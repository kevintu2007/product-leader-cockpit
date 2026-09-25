param(
    [Parameter(Mandatory = $true)]
    [string]$ExecutablePath,

    [Parameter(Mandatory = $true)]
    [string]$ProtectedArtifactPath,

    [int]$ObservationSeconds = 5,

    [int]$PollMilliseconds = 200
)

$ErrorActionPreference = "Stop"
$resolvedExecutable = (Resolve-Path -LiteralPath $ExecutablePath).Path
$repositoryRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..\..")).Path
$commit = (git -C $repositoryRoot rev-parse HEAD).Trim()
$worktreeState = git -C $repositoryRoot status --porcelain
if ($LASTEXITCODE -ne 0) {
    throw "Unable to inspect the Git worktree."
}
if ($worktreeState) {
    throw "Windows smoke evidence must run from a clean reviewed worktree."
}

$buildMetadata = (& $resolvedExecutable --pmc-build-metadata | ConvertFrom-Json)
if ($buildMetadata.commit -ne $commit -or $buildMetadata.dirty -ne "false") {
    throw "Executable provenance does not match the clean reviewed commit."
}

function Get-NetLogSummary {
    param([string]$Path)

    $netLog = Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
    $eventNameById = @{}
    foreach ($property in $netLog.constants.logEventTypes.PSObject.Properties) {
        $eventNameById[[int]$property.Value] = $property.Name
    }
    $urls = @($netLog.events | ForEach-Object { $_.params.url } | Where-Object { $_ } | Sort-Object -Unique)
    $remoteEvents = @($netLog.events | Where-Object { $_.params.remote_address })
    $remoteEndpoints = @($remoteEvents | ForEach-Object { $_.params.remote_address } | Sort-Object -Unique)
    $eventNames = @($netLog.events | ForEach-Object { $eventNameById[[int]$_.type] } | Where-Object { $_ })
    $unexpectedUrls = @($urls | Where-Object {
        $_ -ne "https://chrome.cloudflare-dns.com/dns-query" -and
        $_ -notmatch '^http://tauri\.localhost(?:/|$)'
    })
    $dohEventSourceIds = [System.Collections.Generic.HashSet[int]]::new()
    $dohUrlSourceIds = [System.Collections.Generic.HashSet[int]]::new()
    foreach ($event in $netLog.events) {
        if ($eventNameById[[int]$event.type] -eq "DOH_URL_REQUEST") {
            [void]$dohEventSourceIds.Add([int]$event.source.id)
        }
        if ($event.params.url -eq "https://chrome.cloudflare-dns.com/dns-query") {
            [void]$dohUrlSourceIds.Add([int]$event.source.id)
        }
    }
    $dohSourceIds = [System.Collections.Generic.HashSet[int]]::new()
    $requestJobSourceIds = [System.Collections.Generic.HashSet[int]]::new()
    $approvedSocketSourceIds = [System.Collections.Generic.HashSet[int]]::new()
    foreach ($event in $netLog.events) {
        $sourceId = [int]$event.source.id
        if ($dohEventSourceIds.Contains($sourceId) -and $dohUrlSourceIds.Contains($sourceId)) {
            [void]$dohSourceIds.Add($sourceId)
        }
    }
    foreach ($event in $netLog.events) {
        $eventName = $eventNameById[[int]$event.type]
        $sourceId = [int]$event.source.id
        if ($eventName -eq "HTTP_STREAM_REQUEST_BOUND_TO_JOB" -and
            $dohSourceIds.Contains($sourceId) -and
            $null -ne $event.params.source_dependency.id) {
            [void]$requestJobSourceIds.Add([int]$event.params.source_dependency.id)
        }
    }
    foreach ($event in $netLog.events) {
        $eventName = $eventNameById[[int]$event.type]
        $sourceId = [int]$event.source.id
        if ($eventName -eq "SOCKET_POOL_BOUND_TO_SOCKET" -and
            $requestJobSourceIds.Contains($sourceId) -and
            $null -ne $event.params.source_dependency.id) {
            [void]$approvedSocketSourceIds.Add([int]$event.params.source_dependency.id)
        }
    }
    $unclassifiedRemoteEndpoints = @()
    foreach ($remoteEvent in $remoteEvents) {
        if (-not $approvedSocketSourceIds.Contains([int]$remoteEvent.source.id)) {
            $unclassifiedRemoteEndpoints += [ordered]@{
                sourceId = [int]$remoteEvent.source.id
                remoteAddress = [string]$remoteEvent.params.remote_address
            }
        }
    }
    $quicHousekeepingEvents = @(
        "QUIC_SESSION_POOL_CLOSE_ALL_SESSIONS",
        "QUIC_SESSION_POOL_MARK_ALL_ACTIVE_SESSIONS_GOING_AWAY"
    )
    $quicTransportEvents = @($eventNames | Where-Object {
        $_ -match "QUIC" -and $_ -notin $quicHousekeepingEvents
    })
    return [ordered]@{
        urls = $urls
        remoteEndpoints = $remoteEndpoints
        cloudflareDohRequestSources = $dohSourceIds.Count
        unclassifiedRemoteEndpoints = $unclassifiedRemoteEndpoints
        quicTransportEventCount = $quicTransportEvents.Count
        unexpectedUrls = $unexpectedUrls
        netLogSha256 = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}

$negativeFixturePath = Join-Path $PSScriptRoot "fixtures\webview2\shared-ancestor-netlog.json"
$negativeFixtureSummary = Get-NetLogSummary -Path $negativeFixturePath
if ($negativeFixtureSummary.unclassifiedRemoteEndpoints.Count -ne 1) {
    throw "WebView2 netlog classifier negative fixture did not reject the unrelated remote endpoint."
}

function Get-ProcessTreeIds {
    param([int]$RootProcessId)

    $processes = @(Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId)
    $known = [System.Collections.Generic.HashSet[int]]::new()
    [void]$known.Add($RootProcessId)
    $changed = $true
    while ($changed) {
        $changed = $false
        foreach ($candidate in $processes) {
            if ($known.Contains([int]$candidate.ParentProcessId) -and $known.Add([int]$candidate.ProcessId)) {
                $changed = $true
            }
        }
    }
    return @($known)
}

$runs = @()
foreach ($runNumber in 1..2) {
    $netLogPath = "$ProtectedArtifactPath.run$runNumber.netlog.json"
    Remove-Item -LiteralPath $netLogPath -ErrorAction SilentlyContinue
    $previousBrowserArguments = $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--log-net-log=$netLogPath --net-log-capture-mode=Default"
    $process = Start-Process -FilePath $resolvedExecutable -PassThru -WindowStyle Hidden
    $seenProcessIds = [System.Collections.Generic.HashSet[int]]::new()
    $processIdentityById = @{}
    $allSocketObservations = 0
    $platformRuntimeTransportObservations = @()
    $unexpectedTransportObservations = @()
    $udpEndpointObservations = @()
    $pollCount = [Math]::Ceiling(($ObservationSeconds * 1000) / $PollMilliseconds)

    foreach ($poll in 1..$pollCount) {
        Start-Sleep -Milliseconds $PollMilliseconds
        $process.Refresh()
        if ($process.HasExited) {
            throw "Run $runNumber exited before the observation interval completed."
        }
        $treeIds = @(Get-ProcessTreeIds -RootProcessId $process.Id)
        foreach ($processId in $treeIds) {
            [void]$seenProcessIds.Add([int]$processId)
            if (-not $processIdentityById.ContainsKey([int]$processId)) {
                $processInfo = Get-CimInstance Win32_Process -Filter "ProcessId = $processId" -ErrorAction SilentlyContinue
                $signatureStatus = "Unavailable"
                $signerSubject = ""
                if ($processInfo -and $processInfo.ExecutablePath) {
                    $processSignature = Get-AuthenticodeSignature -FilePath $processInfo.ExecutablePath
                    $signatureStatus = [string]$processSignature.Status
                    if ($processSignature.SignerCertificate) {
                        $signerSubject = [string]$processSignature.SignerCertificate.Subject
                    }
                }
                $processIdentityById[[int]$processId] = [ordered]@{
                    processId = [int]$processId
                    name = [string]$processInfo.Name
                    signatureStatus = $signatureStatus
                    signerSubject = $signerSubject
                    isRootProcess = ([int]$processId -eq $process.Id)
                }
            }
        }
        $connections = @(Get-NetTCPConnection -ErrorAction SilentlyContinue | Where-Object {
            $seenProcessIds.Contains([int]$_.OwningProcess)
        })
        $udpEndpoints = @(Get-NetUDPEndpoint -ErrorAction SilentlyContinue | Where-Object {
            $seenProcessIds.Contains([int]$_.OwningProcess)
        })
        foreach ($udpEndpoint in $udpEndpoints) {
            $udpEndpointObservations += [ordered]@{
                localAddress = [string]$udpEndpoint.LocalAddress
                localPort = [int]$udpEndpoint.LocalPort
                owningProcess = [int]$udpEndpoint.OwningProcess
            }
        }
        $allSocketObservations += $connections.Count
        foreach ($connection in $connections) {
            $transportStates = @("Established", "SynSent", "SynReceived", "FinWait1", "FinWait2", "CloseWait", "Closing", "LastAck", "TimeWait")
            $localOnlyAddresses = @("0.0.0.0", "::", "127.0.0.1", "::1")
            if ($transportStates -contains [string]$connection.State -and $localOnlyAddresses -notcontains [string]$connection.RemoteAddress) {
                $observation = [ordered]@{
                    state = [string]$connection.State
                    remoteAddress = [string]$connection.RemoteAddress
                    remotePort = [int]$connection.RemotePort
                    owningProcess = [int]$connection.OwningProcess
                }
                $identity = $processIdentityById[[int]$connection.OwningProcess]
                $isAcceptedPlatformRuntime = $identity -and
                    -not $identity.isRootProcess -and
                    $identity.name -eq "msedgewebview2.exe" -and
                    $identity.signatureStatus -eq "Valid" -and
                    $identity.signerSubject -match "(?:^|, )O=Microsoft Corporation(?:,|$)" -and
                    [int]$connection.RemotePort -eq 443
                if ($isAcceptedPlatformRuntime) {
                    $platformRuntimeTransportObservations += $observation
                }
                else {
                    $unexpectedTransportObservations += $observation
                }
            }
        }
    }

    try {
        $closeAccepted = $process.CloseMainWindow()
        if (-not $process.WaitForExit(5000)) {
            Stop-Process -Id $process.Id
            throw "Run $runNumber did not close within five seconds."
        }
    }
    finally {
        if ($null -eq $previousBrowserArguments) {
            Remove-Item Env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS -ErrorAction SilentlyContinue
        }
        else {
            $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = $previousBrowserArguments
        }
    }
    if (-not (Test-Path -LiteralPath $netLogPath)) {
        throw "Run $runNumber did not produce the required protected WebView2 netlog."
    }
    $netLogSummary = Get-NetLogSummary -Path $netLogPath
    if ($netLogSummary.unexpectedUrls.Count -ne 0 -or
        $netLogSummary.quicTransportEventCount -ne 0 -or
        $netLogSummary.unclassifiedRemoteEndpoints.Count -ne 0 -or
        $udpEndpointObservations.Count -ne 0) {
        throw "Run $runNumber produced transport outside the approved ADR 0009 Option 2A boundary."
    }
    $runs += [ordered]@{
        run = $runNumber
        observationSeconds = $ObservationSeconds
        pollMilliseconds = $PollMilliseconds
        processTreeIdsObserved = @($seenProcessIds).Count
        processIdentities = @($processIdentityById.Values)
        allSocketPollObservations = $allSocketObservations
        platformRuntimeTransportObservations = $platformRuntimeTransportObservations.Count
        unexpectedTransportObservations = $unexpectedTransportObservations.Count
        udpEndpointObservations = $udpEndpointObservations.Count
        netLog = $netLogSummary
        closeMainWindowAccepted = $closeAccepted
        exitCode = $process.ExitCode
    }
}

$signature = Get-AuthenticodeSignature -FilePath $resolvedExecutable
$evidence = [ordered]@{
    schemaVersion = 1
    reviewedCommit = $commit
    executableSha256 = (Get-FileHash -LiteralPath $resolvedExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
    executableLengthBytes = (Get-Item -LiteralPath $resolvedExecutable).Length
    authenticodeStatus = [string]$signature.Status
    networkObservationScope = "root process and recursively discovered child processes for the full observation interval; Option 2A permits only valid Microsoft-signed WebView2 descendants on TCP 443 whose protected netlog URL is local Tauri content or the exact Cloudflare DNS-over-HTTPS endpoint; QUIC events and all other URLs fail closed"
    executableBuildMetadata = $buildMetadata
    runs = $runs
}

$evidence | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $ProtectedArtifactPath -Encoding utf8
if ($runs.unexpectedTransportObservations | Where-Object { $_ -ne 0 }) {
    throw "At least one process-tree transport fell outside ADR 0009."
}
if ($runs.exitCode | Where-Object { $_ -ne 0 }) {
    throw "At least one smoke run exited unsuccessfully."
}

[ordered]@{
    reviewedCommit = $commit
    executableSha256 = $evidence.executableSha256
    runs = $runs.Count
    platformRuntimeTransportObservations = ($runs.platformRuntimeTransportObservations | Measure-Object -Sum).Sum
    unexpectedTransportObservations = ($runs.unexpectedTransportObservations | Measure-Object -Sum).Sum
    protectedArtifactSha256 = (Get-FileHash -LiteralPath $ProtectedArtifactPath -Algorithm SHA256).Hash.ToLowerInvariant()
} | ConvertTo-Json -Compress

import { type Dispatch, type SetStateAction, useCallback, useEffect, useState } from "react";

import { applyTextScale, readStoredTextScale } from "./overlays/appTextScale";
import { BackupShortcutContext } from "./backup/BackupShortcut";
import type { BackupsSectionProps } from "./backup/BackupsSection";
import { tauriBackupActions, type BackupStatusDto } from "./backup/backupIpc";
import { useBackupStatus } from "./backup/useBackupStatus";
import {
  tauriRestoreActions,
  tauriSystemHealthActions,
  type RestoreResultDto,
} from "./backup/restoreIpc";
import type { PolicyStripState } from "./shell/PolicyStrip";
import { useT } from "./i18n/useT";
import type { Translator } from "./i18n/messages";

import { ExecutiveCockpit } from "./routes/ExecutiveCockpit";
import type { GettingStartedSources } from "./routes/GettingStarted";
import { People } from "./routes/People";
import { Portfolio } from "./routes/Portfolio";
import { Reviews } from "./routes/Reviews";
import { Settings } from "./routes/Settings";
import { SystemHealth } from "./routes/SystemHealth";
import { UpgradeGate } from "./backup/UpgradeGate";
import { tauriUpgradeActions, type UpgradeGateDto } from "./backup/upgradeIpc";
import { tauriVaultRootActions } from "./vault/vaultRootIpc";
import { tauriEvidenceFileActions } from "./evidence/evidenceFileIpc";
import { loadDisplayTimezone, tauriEntryActions } from "./entry/entryIpc";
import { Vault } from "./routes/Vault";
import { type HeldReviews, WorkQueue } from "./routes/WorkQueue";
import { tauriAdapter } from "./adapters/tauriAdapter";
import { AppShell } from "./shell/AppShell";
import { FirstRunGate } from "./workspace/FirstRunGate";
import { SampleBadge } from "./workspace/SampleBadge";
import type { WorkspaceSectionProps } from "./workspace/WorkspaceSection";
import { tauriWorkspaceActions, type WorkspaceStatusDto } from "./workspace/workspaceIpc";
import { useFlaggedWorkCount } from "./useFlaggedWorkCount";
import type { RouteId } from "./shell/routes";

/** The inspector's Evidence writes, with "Add Evidence from a file…". One
 * object for the app's life, so the inspector does not re-read on every render. */
const inspectorEvidenceActions = { ...tauriAdapter, evidenceFile: tauriEvidenceFileActions };

/**
 * Supplies route content to the shared shell.
 *
 * Executive Cockpit (S01), Portfolio (S02), People (S09) and Work Queue (S03)
 * are implemented. Reviews & Reports, Product Vault and Settings show what the
 * host can already answer and say what they cannot. System Health (S11) says
 * whether the Product Ledger is open and, if not, why.
 */
function renderRoute(
  routeId: RouteId,
  navigate: (id: RouteId) => void,
  held: HeldReviews,
  setHeld: Dispatch<SetStateAction<HeldReviews>>,
  onWorkRead: () => void,
  backups: BackupsSectionProps | undefined,
  timeZone: string | undefined,
  workspace: WorkspaceSectionProps | undefined,
  gettingStarted: GettingStartedSources | undefined,
) {
  if (routeId === "executive-cockpit") {
    return (
      <ExecutiveCockpit
        load={tauriAdapter.loadExecutiveCockpit}
        loadDetail={tauriAdapter.loadProductDetail}
        evidenceActions={inspectorEvidenceActions}
        gettingStarted={gettingStarted}
      />
    );
  }
  if (routeId === "portfolio") {
    return (
      <Portfolio
        load={tauriAdapter.loadPortfolioOverview}
        loadDetail={tauriAdapter.loadProductDetail}
        evidenceActions={inspectorEvidenceActions}
        entryActions={tauriEntryActions}
        {...(timeZone === undefined ? {} : { timeZone })}
      />
    );
  }
  if (routeId === "reviews-and-reports") {
    return (
      <Reviews
        load={tauriAdapter.loadExecutiveCockpit}
        onOpenWorkQueue={() => {
          navigate("work-queue");
        }}
      />
    );
  }
  if (routeId === "product-vault") {
    return (
      <Vault
        loadVaultStatus={tauriAdapter.loadVaultStatus}
        loadEvidenceReferences={tauriAdapter.loadEvidenceReferences}
        evidenceFile={tauriEvidenceFileActions}
      />
    );
  }
  if (routeId === "settings") {
    return (
      <Settings
        loadLedgerStatus={tauriAdapter.loadLedgerStatus}
        loadVaultStatus={tauriAdapter.loadVaultStatus}
        backups={backups}
        // Live only, like restore: the Training workspace's Vault is its own
        // synthetic one, and the host refuses to change it either way.
        vaultRoot={backups?.restore === undefined ? undefined : tauriVaultRootActions}
        workspace={workspace}
      />
    );
  }
  if (routeId === "system-health") {
    return <SystemHealth actions={tauriSystemHealthActions} backups={backups} />;
  }
  if (routeId === "people") {
    return <People load={tauriAdapter.loadPeopleDirectory} entryActions={tauriEntryActions} />;
  }
  // The last route: "work-queue".
  return (
    <WorkQueue
      load={tauriAdapter.loadWorkQueue}
      actions={tauriAdapter}
      entryActions={tauriEntryActions}
      {...(timeZone === undefined ? {} : { timeZone })}
      heldReviews={held}
      onHeldReviewsChange={setHeld}
      onRead={onWorkRead}
    />
  );
}

/**
 * The rail badge reads through the same reviewed read-only query the Work
 * Queue itself uses (`onlyFlagged`, one row, so `total` is the flagged count).
 * Module level, so the loader is stable and the refresh it drives is too.
 */
function loadFlaggedCount(): Promise<number> {
  return tauriAdapter.loadWorkQueue(0, 1, [], true).then((queue) => queue.total);
}
/** What the policy strip says about backups (DG3 backup-setup amendment §4–§5). */
function backupPolicyStates(
  status: BackupStatusDto | undefined,
  t: Translator,
  openBackups: () => void,
): PolicyStripState[] {
  if (status?.state === "due") {
    return [
      {
        kind: "backup-due",
        message: t("backup.strip.due"),
        action: { label: t("backup.openBackups"), onSelect: openBackups },
      },
    ];
  }
  if (status?.state === "backing_up") {
    return [{ kind: "backing-up", message: t("backup.strip.running") }];
  }
  if (status?.state === "restoring") {
    return [{ kind: "restoring", message: t("backup.strip.restoring") }];
  }
  return [];
}

/**
 * Which workspace is open is read first (item ⑨): a profile that has never
 * chosen sees only "Choose how to begin", and nothing of any workspace — no
 * Ledger, Work Queue, backup or upgrade read — starts before the choice.
 */
export function App() {
  // The chosen text scale applies to every screen, the first-run one too.
  useEffect(() => {
    applyTextScale(readStoredTextScale());
  }, []);
  const [workspaceStatus, setWorkspaceStatus] = useState<
    WorkspaceStatusDto | "unavailable" | undefined
  >(undefined);
  const refreshWorkspace = useCallback(() => {
    tauriWorkspaceActions.loadWorkspaceStatus().then(setWorkspaceStatus, () => {
      setWorkspaceStatus((previous) =>
        previous === undefined || previous === "unavailable" ? "unavailable" : previous,
      );
    });
  }, []);
  useEffect(refreshWorkspace, [refreshWorkspace]);
  if (workspaceStatus === undefined) {
    return null;
  }
  const workspace = workspaceStatus === "unavailable" ? undefined : workspaceStatus;
  if (workspace?.firstRun === true) {
    return <FirstRunGate actions={tauriWorkspaceActions} />;
  }
  return <OpenWorkspace workspace={workspace} onWorkspaceChanged={refreshWorkspace} />;
}

function OpenWorkspace({
  workspace,
  onWorkspaceChanged,
}: {
  /** `undefined` when the host could not say; Live is assumed, as the host does. */
  readonly workspace: WorkspaceStatusDto | undefined;
  readonly onWorkspaceChanged: () => void;
}) {
  const t = useT();
  const [flagged, refreshFlagged] = useFlaggedWorkCount(loadFlaggedCount);
  const [backupStatus, refreshBackup, setBackupStatus, backupReadFailed] = useBackupStatus(
    tauriBackupActions.loadBackupStatus,
  );
  const [routeRequest, setRouteRequest] = useState<
    { readonly routeId: RouteId; readonly sequence: number } | undefined
  >(undefined);
  const openBackups = useCallback(() => {
    setRouteRequest((previous) => ({
      routeId: "settings",
      sequence: (previous?.sequence ?? 0) + 1,
    }));
  }, []);
  const openPortfolio = useCallback(() => {
    setRouteRequest((previous) => ({
      routeId: "portfolio",
      sequence: (previous?.sequence ?? 0) + 1,
    }));
  }, []);
  const openSystemHealth = useCallback(() => {
    setRouteRequest((previous) => ({
      routeId: "system-health",
      sequence: (previous?.sequence ?? 0) + 1,
    }));
  }, []);
  // An older or newer Ledger replaces the shell with the upgrade gate; any
  // other Ledger that did not open sends the shell to System Health.
  const [gate, setGate] = useState<UpgradeGateDto | "none" | undefined>(undefined);
  useEffect(() => {
    let live = true;
    tauriUpgradeActions.loadUpgradeGate().then(
      (loaded) => {
        if (live) {
          setGate(loaded.state === "none" ? "none" : loaded);
        }
      },
      () => {
        if (live) {
          setGate("none");
        }
      },
    );
    return () => {
      live = false;
    };
  }, []);
  useEffect(() => {
    if (gate !== "none") {
      return;
    }
    let live = true;
    tauriSystemHealthActions.loadSystemHealth().then(
      (health) => {
        if (live && health.ledger !== "ready") {
          openSystemHealth();
        }
      },
      () => undefined,
    );
    return () => {
      live = false;
    };
  }, [gate, openSystemHealth]);
  const restoreFinished = useCallback(
    (result: RestoreResultDto) => {
      if (result.ledger !== "ready") {
        openSystemHealth();
        return;
      }
      if (result.outcome === "failed_before_replacement") {
        refreshBackup();
        return;
      }
      // A different Ledger and settings: every route, and the language,
      // start again from what the host now holds.
      window.location.reload();
    },
    [openSystemHealth, refreshBackup],
  );
  const backups: BackupsSectionProps = {
    actions: tauriBackupActions,
    status: backupStatus,
    onStatus: setBackupStatus,
    refresh: refreshBackup,
    restore:
      backupStatus !== undefined && backupStatus.state !== "not_required"
        ? { actions: tauriRestoreActions, onFinished: restoreFinished }
        : undefined,
  };
  // The workspace's configured time zone, read once for the entry sheets;
  // while it is unknown (or the settings document is unavailable) a sheet
  // falls back to the browser's zone, as the top bar does.
  const [timeZone, setTimeZone] = useState<string | undefined>(undefined);
  useEffect(() => {
    let live = true;
    loadDisplayTimezone().then(
      (zone) => {
        if (live) {
          setTimeZone(zone);
        }
      },
      () => undefined,
    );
    return () => {
      live = false;
    };
  }, []);
  // Reviews closed without deciding outlive the Work Queue route: leaving
  // and returning must lead back to the still-pending preview, never
  // prepare a second one beside it.
  const [held, setHeld] = useState<HeldReviews>(() => new Map());
  if (gate === undefined) {
    return null;
  }
  const sampleOpen = workspace?.open === "training";
  // Backups and restore are not offered in the sample workspace (§6); the
  // host refuses them there too.
  const liveBackups = sampleOpen ? undefined : backups;
  if (gate !== "none") {
    return (
      <>
        {sampleOpen && <SampleBadge />}
        <UpgradeGate
          gate={gate}
          actions={tauriUpgradeActions}
          backups={{ ...backups, restore: undefined }}
          onUpgraded={() => {
            // An upgraded Ledger: every route starts from what it now holds.
            window.location.reload();
          }}
          onOpenSystemHealth={() => {
            setGate("none");
          }}
          onSwitchToLive={
            sampleOpen ? () => tauriWorkspaceActions.switchWorkspace("live") : undefined
          }
          // Live only, as in Settings (restore-unopened amendment §2).
          restore={
            backups.restore === undefined || sampleOpen
              ? undefined
              : {
                  actions: tauriRestoreActions,
                  onFinished: (result) => {
                    // A Ledger that does not open, or that a failed recovery
                    // left uncertain: System Health says what happened. Any
                    // other outcome — open, waiting for an upgrade, or the
                    // gate's own Ledger put back — starts again from what the
                    // host now holds.
                    if (
                      result.ledger === "open_failed" ||
                      result.ledger === "restore_recovery_required"
                    ) {
                      setGate("none");
                      return;
                    }
                    window.location.reload();
                  },
                }
          }
        />
      </>
    );
  }
  // Your own workspace only (getting-started amendment §1): shown only when
  // the host says so — an unreadable workspace status claims nothing.
  const gettingStarted: GettingStartedSources | undefined =
    workspace?.open !== "live"
      ? undefined
      : {
          backup: backupStatus,
          backupReadFailed,
          loadVaultStatus: tauriAdapter.loadVaultStatus,
          onOpenSettings: openBackups,
          onOpenPortfolio: openPortfolio,
        };
  const workspaceSection: WorkspaceSectionProps | undefined =
    workspace === undefined
      ? undefined
      : { status: workspace, actions: tauriWorkspaceActions, onChanged: onWorkspaceChanged };
  return (
    <BackupShortcutContext.Provider value={openBackups}>
      <AppShell
        renderRoute={(routeId, navigate) =>
          renderRoute(
            routeId,
            navigate,
            held,
            setHeld,
            refreshFlagged,
            liveBackups,
            timeZone,
            workspaceSection,
            gettingStarted,
          )
        }
        navCounts={flagged === undefined ? undefined : { "work-queue": flagged }}
        policyStates={sampleOpen ? [] : backupPolicyStates(backupStatus, t, openBackups)}
        routeRequest={routeRequest}
        workspaceBadge={sampleOpen ? <SampleBadge onOpen={openBackups} /> : undefined}
      />
    </BackupShortcutContext.Provider>
  );
}

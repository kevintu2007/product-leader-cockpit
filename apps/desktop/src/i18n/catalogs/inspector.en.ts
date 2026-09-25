/** English copy for O01, the Product inspector. */
export const INSPECTOR_EN = {
  "inspector.route": "Product detail",
  "inspector.outOfSync":
    "The two sources of this Product's detail read different Ledger revisions, so nothing is shown. Joining two moments into one inspector would look right and be wrong.",
  "inspector.healthEvidence": "Evidence {label} is linked; verification: {verification}.",
  "inspector.healthRecord": "{kind} “{label}”: {reason}.",

  "inspector.pinConfirm":
    "Pin a fingerprint for {id}? The app reads the file this Evidence is at now and records a digest of its bytes as its identity. A pin is permanent: the same content later is a “re-observation”, a moved file is a “relocation”, changed content is a “supersession”; none of them rewrites this fingerprint.",
  "inspector.reobserveConfirm":
    "Re-observe {id}? The app reads the file at the location this Evidence records, and writes only if what it sees differs from what is stored.",
  "inspector.confirmPin": "Confirm pin",
  "inspector.confirmReobserve": "Confirm re-observation",
  "inspector.cancel": "Cancel",
  "inspector.sending": "Sending…",
  "inspector.written": "Written: the verification is now {verification}.",
  "inspector.unchanged":
    "No change: what was observed matches what is stored, so the Ledger wasn't written.",
  "inspector.close": "Close",
  "inspector.pinFailed": "The pin didn't finish. {message}",
  "inspector.reobserveFailed": "The re-observation didn't finish. {message}",
  "inspector.abandon": "Give up this action",
  "inspector.pin": "Pin fingerprint",
  "inspector.reobserve": "Re-observe",

  "inspector.link": "Link Evidence to this Product",
  "inspector.linkLoading": "Reading Evidence…",
  "inspector.linkChoose": "Evidence to link to {product}",
  "inspector.linkNone": "(no Evidence left to link)",
  "inspector.linkCandidate": "{id} ({verification}, {classification}, version {version})",
  "inspector.linkConfirm": "Confirm link",
  "inspector.linked": "Linked {id}; classification at link: {classification}.",
  "inspector.linkFailed": "The link didn't finish. {message}",

  "inspector.classification": "Classification",
  "inspector.version": "Version",
  "inspector.fold":
    "This inspector is shown as {classification} because the {kind} {id} it shows has that classification.",
  "inspector.happened": "What happened",
  "inspector.nothingHappened": "Nothing needs attention right now.",
  // {provenance} is the muted source note, as its own element.
  "inspector.conditionLine": "{condition} {provenance}",
  "inspector.provenance": "Source: {owner} {id}, version {version}",
  "inspector.impact": "Impact",
  "inspector.impactUnassessed": "Not assessed by anyone yet",
  "inspector.tabs": "Product detail",
  "inspector.tab.structure": "Structure",
  "inspector.tab.evidence": "Evidence",
  "inspector.tab.people": "People",
  "inspector.structureNone": "Nothing in the Ledger is structured around this Product.",
  "inspector.structureEntry": "{kind}: {label} ({classification})",
  "inspector.structureEntryVia": "{kind}: {label} ({classification}), {via}",
  "inspector.via": "via {project}",
  "inspector.vaultNotConfigured":
    "This workspace has no Product Vault yet (not set). Actions that read files — pin fingerprint, re-observe — aren't available; linking Evidence still works.",
  "inspector.vaultUnavailable":
    "The Product Vault can't be read right now (the folder is missing, isn't a folder, or is a link). Actions that read files — pin fingerprint, re-observe — are paused; linking Evidence still works.",
  "inspector.evidenceNone": "No Evidence is linked directly to this Product.",
  "inspector.evidenceLine": "{id}: {verification} {classifications}",
  "inspector.evidenceLineUnpinned": "{id}: {verification} {unpinned} {classifications}",
  "inspector.unpinned": "(no pinned fingerprint)",
  "inspector.evidenceClassifications":
    "(Evidence classification {classification}; classification at link {atLink})",
  "inspector.peopleNone": "Nobody in the Ledger is responsible for this Product or depends on it.",
  "inspector.person.one": "{name} ({purpose}; also tied to {count} other Product)",
  "inspector.person.other": "{name} ({purpose}; also tied to {count} other Products)",
  "inspector.purpose.responsibility": "responsible",
  "inspector.purpose.dependency": "depends on it",
  "inspector.carriedHeading":
    "Carried by this person now (through their responsibility, not owned by this Product)",
  "inspector.carriedNone": "Not carrying any work items right now.",
  "inspector.carriedLine": "{kind} {label} ({state}){attention} {intents}",
  "inspector.nextSteps": "Next steps the lifecycle allows: {intents}",
  "inspector.noNextSteps": "none",
} as const;

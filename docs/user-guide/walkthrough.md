# Product Mission Control: user guide

Version 0.2.0-beta · [繁體中文](walkthrough.zh-TW.md)

This guide has four parts:

1. [Install and first run](#part-1--install-and-first-run)
2. [A week on sample data](#part-2--a-week-on-sample-data), one action and one screenshot per step
3. [Start your own workspace](#part-3--start-your-own-workspace)
4. [Uninstall](#part-4--uninstall)

Every screenshot was taken in the real app, of the PMC window only. File and folder choosers belong
to Windows, so they appear here as written steps, never as pictures. Where something cannot be done
on screen yet, the guide says so.

---

## Part 1 — Install and first run

### Download and check the installer

From the release page, download two files:

- `product-mission-control_0.2.0-beta_windows-x86_64_nsis-setup.exe`
- `product-mission-control_0.2.0-beta_windows-x86_64_nsis-setup.exe.sha256`

Before you run the installer, check that it is the file that was published. In PowerShell, in the
folder you downloaded to:

```powershell
(Get-FileHash .\product-mission-control_0.2.0-beta_windows-x86_64_nsis-setup.exe -Algorithm SHA256).Hash.ToLower()
Get-Content .\product-mission-control_0.2.0-beta_windows-x86_64_nsis-setup.exe.sha256
```

The two 64-character values must be identical. If they are not, delete the download and do not run
it.

### Run the installer

The beta installer is not code-signed, so Windows SmartScreen warns that it does not recognise the
app. Choose **More info**, check that the file name is the one above, then choose **Run anyway**.

The installer installs for your Windows account only (no administrator rights needed), into
`%LOCALAPPDATA%\Product Mission Control`, and adds a Start menu entry. If WebView2 is missing, the
installer includes it; Windows 11 already has it.

### Choose how to begin

1. Start **Product Mission Control**. The first screen asks how you want to begin. The two choices
   are equal; you can switch later in Settings → Workspace.

   ![Choose how to begin: Start with my workspace, or Learn with sample data](images/en/s01-first-run.png)

   - **Start with my workspace** opens an empty workspace. Your records stay on this computer. Part 3
     of this guide sets it up.
   - **Learn with sample data** opens a synthetic workspace to practise in. It is separate from your
     work, and you can reset or delete it at any time.

2. Choose **Learn with sample data**. PMC prepares the sample data, which can take a minute, and
   then restarts by itself.

   ![Preparing the sample data](images/en/s02-preparing.png)

3. PMC opens on the **Executive Cockpit** of the sample workspace. You can always tell where you
   are: the window title ends in **— Sample data**, and the top bar shows **Sample workspace**
   (selecting it opens Settings → Workspace).

   ![The sample workspace: Executive Cockpit, with the Sample workspace badge in the top bar](images/en/s03-sample-cockpit.png)

4. The language follows Windows. To choose another, open **Settings** and pick one of the six
   languages under **Language**: English, 繁體中文, 简体中文, 日本語, 한국어 or Español.

   ![Settings: Workspace and Language](images/en/s04-language.png)

---

## Part 2 — A week on sample data

You play the Head of Products for one week. Each step is one action and one screenshot, in the order
they were taken. Every change goes through the same pattern: **prepare** a preview, read exactly what
approving will do, then **approve** or **reject**. A rejection is recorded; closing a preview without
deciding is not.

### What the sample data contains

| Kind             | Contents                                                                                                                                                                                                           |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Product          | Demo Product Atlas, Beacon, Cinder, Delta                                                                                                                                                                          |
| Stakeholder      | Six `Demo Person: …` and two `Demo Org: …` (the hosting vendor is Confidential)                                                                                                                                    |
| Action Request   | Six, all open. Three are past their response date; two have no owner                                                                                                                                               |
| Decision Request | Three, all open                                                                                                                                                                                                    |
| Risk             | Four, all open                                                                                                                                                                                                     |
| Issue            | Four, all open. The fourth is titled "second occurrence" to show repeated work; no recurrence link is stored                                                                                                         |
| Evidence         | `-1` verified; `-2` readable but no pinned fingerprint; `-3` verified before but can't be confirmed now; `-4` not verified; `-5` verified (Restricted); `-6` verified (linked to an Action Request, not a Product) |

### Monday: look, don't touch

Nothing is written today.

1. PMC opens on the **Executive Cockpit**. The Portfolio Lens places every Product by milestone dates
   (across) and outcome observability (up). It does not rank Products and does not estimate
   confidence. All four sample Products sit in **Monitor closely**.

   ![Executive Cockpit: all four Products in Monitor closely](images/en/01-cockpit.png)

2. Select **Demo Product Beacon** in the chart. The right side lists its Lens measures, then the
   Product's health: **What happened** lists what needs attention now, each line naming the record
   and version it came from.

   ![Beacon selected: its Lens measures and What happened](images/en/02-cockpit-select.png)

3. Select **Evidence** at the top right of the Lens. A mode changes only the marking and the
   explanation; no Product moves. This mode marks Products whose weakest Evidence doesn't match its
   record or hasn't been verified. None is marked: Beacon's Evidence is readable but has no pinned
   fingerprint.

   ![Evidence mode: no Product marked; Beacon has 0/1 verified](images/en/03-cockpit-evidence-mode.png)

4. Select **Portfolio** in the left rail. Products are sorted by name, and the screen says so: an
   order to browse by, not a priority. Each row carries the same Lens measures plus the flagged work
   of the people responsible and the classification. **View**, at the right end of the row, opens
   the same health panel.

   ![Portfolio: the four Products with milestones, outcome observability, verified Evidence, quadrant and flagged work](images/en/04-portfolio.png)

5. Select **People**. Each Stakeholder shows what they are responsible for, what they depend on and
   which requests are still waiting on their answer. Stakeholders are records you keep, not user
   accounts.

   ![People: who is responsible for what, and who is still waiting on an answer](images/en/05-people.png)

6. Select **Reviews & Reports**. No review period has been approved yet, so there is nothing to
   compare against. The right side lists the five items the Work Queue has flagged, in the Cockpit's
   order of attention; **Act on it in the Work Queue** takes you there.

   ![Reviews & Reports: no review period yet; five flagged items](images/en/06-reviews.png)

7. Select **Product Vault**. The Vault is available, and the table lists the six Evidence files with
   verification, fingerprint, classification and version. The screen never shows file paths.

   ![Product Vault: six Evidence files and where they stand](images/en/07-vault.png)

8. Select **Settings**. Text size changes only this computer's screen, and the example below it
   follows. **Data sources** shows the Ledger's schema version and current version, and whether the
   Vault is available. Switch between light and dark with the button at the top right.

   ![Settings: Workspace, Language, Text size and Data sources](images/en/08-settings.png)

### Tuesday: Action Requests

9. Select **Work Queue**. Every row says what needs attention, the deadline, why it is placed there,
   and the next steps its state allows. The kind buttons filter the list; the number in brackets is
   how many you will see.

   ![Work Queue: 17 items, led by Action Requests past their response date](images/en/09-work-queue.png)

10. Select the title **Confirm the Atlas rollout window** to open its detail. The buttons there are
    the same as in the row.

    ![Action Request detail: the response is overdue; next steps Prepare to accept, Decline, Withdraw](images/en/10-item-detail.png)

11. Select **Prepare to accept**. The review sheet opens. Check each line: the record and version to
    change, the new Action id the app assigned, subject, commitment, owner, due date, what approving
    does, and where the classification comes from.

    ![Review sheet: accept Confirm the Atlas rollout window](images/en/11-review-sheet.png)

12. Scroll down. **Digest of what you approve** is the full 64-character digest. **Preview valid
    until** shows the time and the minutes left; a preview is valid for five minutes.

    ![Review sheet, lower half: the digest, the preparation record and the time left](images/en/12-review-sheet-digest.png)

13. Select **Approve and accept**. The sheet reports the Action it created, the Request it linked and
    the receipt number.

    ![Approved and carried out: the Action created and linked, with its receipt](images/en/13-approved.png)

    The Ledger now holds the Request at its next version, the new Action, a receipt that can be used
    only once, and the audit records.

14. Select **Close**. The list reads again: **Action (1)**, and the Work Queue badge in the rail
    drops by one.

    ![The Work Queue after approving: Action (1)](images/en/14-queue-after-approve.png)

15. For **Approve the Beacon pricing experiment**, select **Prepare to accept**, and this time select
    **Reject**. The sheet says **Rejected.**, and approving is no longer possible.

    ![Rejected: the approve button is disabled](images/en/15-rejected.png)

    A rejection is recorded: the preview is used up and an audit record with no effect is written.
    There is no receipt, and no record changes.

16. Closing a sheet is not rejecting. For **Review the Cinder vendor quote**, select **Prepare to
    accept**, then press Escape, select outside the sheet or select **Decide later**. Nothing is
    recorded. The row now shows **Back to the review**, which returns you to the same preview; the
    row's other buttons wait until you decide.

    ![Decide later: the row shows Back to the review](images/en/16-held-review.png)

17. Leave a preview for more than five minutes and it expires. Approving is disabled; you can still
    reject it, or select **Prepare again** for a fresh preview.

    ![An expired preview: it can be rejected or prepared again](images/en/16b-expired.png)

18. Open **Assign an owner for the partner onboarding kit**, which has no owner, and select **Prepare
    to accept**. PMC refuses: a required field is missing, and trying again will not change the
    result. The message carries a Correlation ID you can copy.

    ![No owner: Prepare to accept is refused, with the reason and a Correlation ID](images/en/17-no-owner-refused.png)

19. Select **Give up this action**, then **Decline**, and write the reason.

    ![The decline form with a reason](images/en/18-decline-form.png)

20. Select **Confirm decline**. The list reports the Request declined, and it leaves the list.

    ![After declining: the confirmation and the shorter list](images/en/19-declined.png)

    Declining and withdrawing both need a reason. The Ledger records it and moves the Request to its
    next version.

### Wednesday: the life of an Action

21. Select **Action**, find the Action created on Tuesday, and select **Start**. It is now in
    progress, and its next steps are **Link Evidence**, **Prepare to complete** and **Prepare to
    cancel**.

    ![Started: the Action is in progress](images/en/20-action-started.png)

22. Select **Link Evidence**, choose `demo-evidence-1`, and select **Confirm link**. The list shows
    each Evidence's verification, fingerprint, classification and version, never a file path.

    ![Link Evidence: demo-evidence-1 (verified, pinned, Internal, version 1)](images/en/21-link-evidence.png)

23. Select **Prepare to complete**. Leaving the Judgment rationale empty attaches none; it is required
    if the linked Evidence isn't verified.

    ![The completion form, Judgment rationale empty](images/en/22-complete-form.png)

24. Select **Prepare the completion preview**. **Evidence and judgment** says the Evidence is
    sufficient and lists its source version and digest. If the Evidence is moved, re-observed or
    pinned before you approve, the digest changes and approving is refused.

    ![The completion review: Evidence is sufficient](images/en/23-complete-sheet.png)

25. Select **Approve and complete**. The sheet reports the Action completed.

    ![Approved and carried out: the Action is completed](images/en/24-completed.png)

26. Accept and start another Action the same way (steps 11–13 and 21). Link `demo-evidence-2`, which
    is readable but has no pinned fingerprint, and select **Prepare to complete** without a Judgment.
    PMC refuses.

    ![Prepare to complete refused: the Evidence doesn't support it on its own](images/en/25-judgment-required.png)

27. Select **Prepare to complete** again, and this time write a Judgment rationale and choose its
    classification.

    ![The completion form with a Judgment rationale](images/en/26-judgment-form.png)

28. Select **Prepare the completion preview**. It now says the Evidence is waiting to be verified and
    PMC is proceeding on your judgment, which it lists.

    ![The completion review: proceeding on human judgment](images/en/27-judgment-sheet.png)

29. Select **Approve and complete**.

    ![Approved and carried out: completed on a Judgment](images/en/28-judgment-completed.png)

30. Start a third Action and link `demo-evidence-4`, which has never been verified. Even with a
    Judgment, **Prepare to complete** is refused.

    ![Evidence that was never verified: completion is refused even with a Judgment](images/en/29-no-rescue.png)

    The difference is whether there is something to judge. Evidence that is readable but unpinned, or
    was verified before and can't be confirmed now, can be carried by your written judgment. Evidence
    that was never verified, or doesn't match its record, cannot be vouched for by a judgment.

31. Select **Give up this action**, then **Prepare to cancel**, write the reason, prepare the preview
    and select **Approve and cancel**. The sheet reports the Action cancelled.

    ![Approved and carried out: the Action is cancelled](images/en/30-cancelled.png)

32. Select **Prepare to reopen**, keep the mode **Restart a cancelled Action**, and write the reason.

    ![The reopen form: mode and reason](images/en/31-reopen-form.png)

33. Select **Prepare the reopening preview** and check it.

    ![The reopen review](images/en/31b-reopen-sheet.png)

34. Select **Approve and reopen**. The Action is in progress again.

    ![Approved and carried out: the Action is in progress again](images/en/32-reopened.png)

    Completion, cancellation and reopening previews can all be rejected, and a rejection is recorded
    in the Ledger.

### Thursday morning: a Decision Request

35. Select **Decision Request**. For **Choose the Atlas data-residency region**, select **Prepare to
    resolve**. Write the decision, rationale and impact, tick the supporting Evidence
    (`demo-evidence-1`), then select **Add a follow-up Action Request** and fill in its subject,
    details, owner, due date and classification. The app assigns the follow-up's id.

    ![The resolution form: a follow-up Action Request with owner demo-stakeholder-2 and due date 2026-10-31](images/en/33-decision-form.png)

36. Select **Prepare the resolution preview**. The sheet lists the Decision to create, each follow-up
    Action Request with the id the app assigned, what approving does, where each classification comes
    from, and the Evidence the decision rests on.

    ![The resolution review: follow-up, records to change, what approving does and the Evidence](images/en/34-decision-sheet.png)

37. Select **Approve and resolve**. The sheet reports the Decision, one follow-up Action Request and
    the receipt.

    ![Approved and carried out: the Decision and one follow-up Action Request](images/en/35-decision-resolved.png)

    In one transaction the Ledger closes the Decision Request and records the Decision, the follow-up
    Request, a receipt and the audit records.

38. Select **Action Request**. The follow-up **Move the Atlas data store to the EU region** is there,
    and can be accepted, declined or withdrawn.

    ![Work Queue: the new Action Request Move the Atlas data store to the EU region](images/en/36-follow-up-request.png)

    Its Deadline column says **No response deadline**: a follow-up has a promised completion date,
    shown right below, but no date by which it must be answered.

### Thursday afternoon: a Risk

39. Select **Risk**. Each Risk offers **Prepare to record occurrence**, **Prepare to close** and
    **Update the response…**.

    ![Work Queue, Risks only](images/en/37-risk-filter.png)

40. For **Key vendor concentration**, select **Prepare to record occurrence**. There is nothing to
    fill in. The line to read is **Issue to create**: the app assigned that id, and it is the one that
    will be written.

    ![Record occurrence: the Issue to create and what approving does](images/en/38-risk-occurrence-sheet.png)

41. Select **Approve and record occurrence**. The sheet reports the occurrence recorded and the Issue
    created.

    ![Approved and carried out: the occurrence recorded and the Issue created](images/en/39-risk-occurred.png)

    If you reject instead, preparing again gives a **different** Issue id. **Prepare to close** asks
    for a reason, then works the same way: preview, then approve or reject.

### Thursday evening: an Issue

42. Select **Issue**. There are five open Issues, each offering **Prepare to resolve**. **Key vendor
    concentration** is the one the Risk just created, with the Risk's title. **Nightly export fails on
    large accounts (second occurrence)** carries that phrase in its title to show repeated work; this
    release stores no recurrence link between Issues.

    ![Work Queue, Issues only: five open](images/en/40-issue-list.png)

43. For **Onboarding email lands in spam**, select **Prepare to resolve**. Choose the resolution
    (Resolved, Worked around, or Accepted the impact), write the reason and tick `demo-evidence-1`.
    All three resolutions need Evidence.

    ![The resolve form: Resolved, a reason, demo-evidence-1 ticked](images/en/41-issue-resolve-form.png)

    Ticking `demo-evidence-4` (never verified) is refused. Ticking `-2` or `-3` needs a Judgment
    rationale and its classification, and the preview then says it is proceeding on human judgment.

44. Select **Prepare the resolution preview** and check it.

    ![The resolution review for the Issue](images/en/41b-issue-resolve-sheet.png)

45. Select **Approve and resolve**. The Issue is now Resolved.

    ![Approved and carried out: demo-issue-2 is now Resolved](images/en/42-issue-resolved.png)

46. Select **Close**. The Issue now offers **Prepare to close** (needs Evidence: verified, or partly
    verified and carried by a written Judgment) and **Prepare to reopen** (needs Evidence showing that the earlier resolution
    did not hold, plus your reason; that Evidence must itself pass the same Evidence-or-Judgment gate).

    ![A resolved Issue: its state allows Close and Reopen](images/en/43-issue-next-steps.png)

### Friday: Evidence

47. Select **Portfolio**, select **View** for **Demo Product Beacon**, and switch the health panel to
    the **Evidence** tab. `demo-evidence-2` offers **Pin fingerprint** and **Re-observe**. Below the
    list are **Link Evidence to this Product** and **Add Evidence from a file…**.

    ![Beacon's Evidence tab: demo-evidence-2, readable but no pinned fingerprint](images/en/44-evidence-tab.png)

48. Select **Pin fingerprint**. The confirmation explains that a pin is permanent and names the
    Evidence by id; no file path is shown.

    ![Pin confirmation: a pin is permanent](images/en/45-pin-confirm.png)

49. Select **Confirm pin**. `demo-evidence-2` is now Verified, and the Lens measures and the Portfolio
    table read again: Beacon shows 1/1 verified, and What happened no longer lists this Evidence.

    ![After pinning: demo-evidence-2 is Verified; Beacon has 1/1 verified](images/en/46-pinned.png)

50. Select **Close**, select **View** for **Demo Product Atlas**, and select **Re-observe** for
    `demo-evidence-1`. PMC reads the file at the location this Evidence records, and writes only if
    what it sees differs from what is stored.

    ![Re-observe confirmation: demo-evidence-1](images/en/47-reobserve-confirm.png)

51. Select **Confirm re-observation**. The result: **Written: the verification is now Verified.**

    ![After re-observing: written, Verified](images/en/48-reobserved.png)

    A readable file whose content still matches is written every time, because each observation
    records a new time. **No change** appears only when the observation matches what is stored
    exactly — for example, the file is still missing and is already recorded that way.

52. Select **Close**, then **Link Evidence to this Product**. The list offers only Evidence not yet
    linked to Atlas.

    ![The link form: demo-evidence-2 (Verified, Internal, version 3)](images/en/49-link-form.png)

53. Select **Confirm link**. The tab now lists `demo-evidence-2` with its classification at link, and
    Atlas shows 2/2 verified in the panel and the Portfolio table.

    ![After linking: Atlas has two verified Evidence files](images/en/50-linked.png)

### Also on Friday: new records

54. Records are created where they belong. In **Portfolio**, select **New Product…**, write the name
    and details, and choose a classification.

    ![New Product: name, details and classification](images/en/51-new-product.png)

55. Select **Create**. The page reports the new Product and its id, and the table now has five
    Products.

    ![Created: the new Product](images/en/51b-product-created.png)

    New Portfolio, Initiative, Project, Milestone, Roadmap, KPI, Stakeholder, Action Request, Decision
    Request, Risk and Issue work the same way, each from its own screen.

56. Evidence starts as a file in your Vault folder. On a Product's **Evidence** tab, select **Add
    Evidence from a file…**, then **Choose file…**. Windows opens its file chooser: pick a file inside
    the Vault folder (a file outside it is refused) and select **Open**. PMC shows the file name and
    when it observed it; choose a classification.

    ![Evidence from a file: the chosen file, when it was observed, and its classification](images/en/52-evidence-from-file.png)

57. Select **Create and link to this Product**. PMC records where the file is and its fingerprint; the
    file stays where it is.

    ![Created: the new Evidence, linked to Demo Product Atlas](images/en/52b-evidence-created.png)

### Good to know

- **Every error** carries a Correlation ID you can copy. When an error can be retried, **Retry**
  reuses the same request id, so a retry never becomes a second write.
- **Every governed change** can be left before approval: **Reject** is recorded; Escape, selecting
  outside the sheet or **Decide later** only closes it, and **Back to the review** returns to the same
  preview.
- **When the Vault is unavailable**, pinning and re-observing are disabled with the reason; linking
  still works.
- **Relocating** an Evidence file to a new place cannot be done on screen yet.

---

## Part 3 — Start your own workspace

Your workspace is where your real records live. It accepts new records only after a backup has
verified, so the setup order matters. Until you are done, the **Getting started** list at the top of
the Executive Cockpit shows where you are.

1. In **Settings → Workspace**, select **Switch to my workspace**, then **Switch and restart**. PMC
   restarts in your workspace. (From the first-run screen, **Start with my workspace** does the same.)

   ![Switch to my workspace: PMC will restart](images/en/p01-switch-confirm.png)

2. Your workspace opens empty. The **Backup due** strip explains that new records and changes are
   accepted again after a backup, and **Getting started** lists the five setup steps in order. Each
   step is checked from what PMC can actually see; only the next step has a button.

   ![An empty workspace: the Backup due strip and the Getting started list](images/en/p02-live-empty.png)

3. Select **Open Settings**. Under **Backups**, select **Choose folder…**. Windows opens its folder
   chooser: pick a folder, ideally on another drive, and select **Select Folder**. The folder shows as
   **Set** and **Available**.

   ![Backups: the backup folder is set](images/en/p03-backup-folder-set.png)

4. Select **Set up passphrase…**. PMC generates a passphrase and shows it **once**. Write it down or
   keep it in a password manager, then type it in **Type it exactly**. (Select **Use my own instead**
   to choose your own.) Tick that you understand PMC cannot recover it.

   ![Set up the recovery passphrase: shown once](images/en/p04-passphrase.png)

   **Remember on this Windows account for automatic backups** keeps the passphrase in Windows
   Credential Manager so backups can run without asking. Leave it unticked and the passphrase is kept
   only until PMC closes. Select **Use this passphrase**.

   ![The passphrase typed back, and the acknowledgement ticked](images/en/p04b-passphrase-typed.png)

   Backups are encrypted with this passphrase. **Without it, a backup cannot be restored — on this
   computer or any other — and PMC cannot recover a lost passphrase.**

5. Select **Back up now**. When the backup has been written and read back, **Last backup** shows when
   it was verified and when the next one is due, and the Backup due strip goes away.

   ![A verified backup, and the next one due in a day](images/en/p05-backup-verified.png)

   A backup holds the Ledger and PMC's non-secret settings. It does not include your Vault or
   Evidence files; back up that folder separately.

6. Further down Settings, under **Data sources → Product Vault**, select **Choose Vault folder…**. PMC
   explains that it backs up this workspace before anything changes; select **Choose Vault folder…**
   again, and Windows opens its folder chooser: pick the folder that holds (or will hold) your
   Evidence files. PMC then shows exactly what will change — the folder now and the new one, the
   Evidence affected, and the backup it has just made. Type the confirmation phrase it shows and select
   **Use this folder**.

   ![The Vault folder change: what changes, and the backup made first](images/en/p06-vault-confirm.png)

   The Vault is now **Available**, and the button reads **Change Vault folder…**.

   ![The Vault folder is set](images/en/p07-vault-set.png)

7. Back on the Executive Cockpit, Getting started shows four steps done and **Add your first
   Product** next. Select **Open Portfolio** and create your first records: **New Portfolio…**,
   then **New Product…**.

   ![Your first Portfolio and Product](images/en/p08-first-product.png)
   When the first Product exists, all five steps are done and Getting started disappears; the Product
   appears on the Portfolio Lens.

   ![The Executive Cockpit after setup: Getting started is gone](images/en/p08b-cockpit-complete.png)

8. Put a file in your Vault folder, open the Product's **Evidence** tab, and use **Add Evidence from a
   file…** as in step 56 of Part 2.

   ![Evidence created from a file in your Vault](images/en/p09-evidence-created.png)

9. To restore, select **Restore from a backup…** in Settings → Backups, then **Choose backup file…**.
   Windows opens its file chooser: pick an Operational Backup (a `.tar.zst.age` file) from your backup
   folder or another computer's. Type **the passphrase for this backup** and select **Check the
   backup**.

   ![Restore: the chosen backup and its passphrase](images/en/p10a-restore-passphrase.png)

   Before showing anything, PMC backs up the current workspace. The preview then says exactly what the
   restore does: when the backup was made and how many records it holds, what is here now, what is
   replaced (the Product Ledger and the settings it carries), what is not (the Product Vault, the
   backup folder and passphrase settings, every other backup), and the recovery backup it has just
   made. To go ahead you type the backup's creation date and select **Replace with this backup**;
   **Don't restore** leaves the current workspace as it is; the recovery backup already made stays.

   ![The restore preview](images/en/p10-restore-preview.png)

10. The sample data stays available while you work. From your workspace, **Settings → Workspace**
    offers **Reset sample data…**, which returns it to its starting state, and **Delete sample
    data…**, which lists exactly what it removes and asks you to type **DELETE SAMPLE DATA**. Neither
    touches your workspace, settings or backups.

    ![Reset sample data: confirm](images/en/p11-reset-confirm.png)

    ![Delete sample data: exactly what is removed](images/en/p12-delete-sheet.png)

11. **System Health**, at the bottom of the rail, says whether PMC can read and keep your records.

    ![System Health: the Product Ledger is open and readable](images/en/p13-system-health.png)

---

## Part 4 — Uninstall

Uninstall from **Windows Settings → Apps → Installed apps → Product Mission Control → Uninstall**.

Uninstalling removes the program and its Start menu entry. **It does not delete your data.** These
stay where they are:

| What                                 | Where                                              |
| ------------------------------------ | -------------------------------------------------- |
| Your records (Ledger) and the sample | `%LOCALAPPDATA%\ProductMissionControlDesktop`      |
| The app's WebView2 data              | `%LOCALAPPDATA%\com.productmissioncontrol.desktop` |
| Your backups                         | the backup folder you chose                        |
| Your Evidence files                  | the Vault folder you chose                         |

Installing again later opens the same records. To remove your data as well, delete those folders
yourself after uninstalling — and keep a backup and its passphrase if you may want the records back.

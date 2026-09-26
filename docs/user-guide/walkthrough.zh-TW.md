# Product Mission Control 使用手冊

版本 0.2.0-beta · [English](walkthrough.md)

本手冊分四部分：

1. [安裝與首次啟動](#第一部安裝與首次啟動)
2. [範例資料的一週](#第二部範例資料的一週)：一步一個動作、一張截圖
3. [開始使用自己的工作區](#第三部開始使用自己的工作區)
4. [解除安裝](#第四部解除安裝)

所有截圖都在真實 app 中拍攝，只擷取 PMC 視窗。選擇檔案與資料夾的視窗屬於 Windows，本手冊以文字說明，不放截圖。畫面上還不能做的事，會直接寫出來。

---

## 第一部：安裝與首次啟動

### 下載並核對安裝程式

從發布頁下載兩個檔案：

- `product-mission-control_0.2.0-beta_windows-x86_64_nsis-setup.exe`
- `product-mission-control_0.2.0-beta_windows-x86_64_nsis-setup.exe.sha256`

執行前，先確認它就是發布的那個檔案。在下載資料夾開 PowerShell：

```powershell
(Get-FileHash .\product-mission-control_0.2.0-beta_windows-x86_64_nsis-setup.exe -Algorithm SHA256).Hash.ToLower()
Get-Content .\product-mission-control_0.2.0-beta_windows-x86_64_nsis-setup.exe.sha256
```

兩個 64 字元的值必須完全相同。不同的話，刪掉下載的檔案，不要執行。

### 執行安裝程式

beta 版安裝程式沒有程式碼簽章，所以 Windows SmartScreen 會警告它不認得這個 app。按 **其他資訊**，確認檔名是上面那個，再按 **仍要執行**。

安裝程式只為你的 Windows 帳戶安裝（不需要系統管理員權限），裝到 `%LOCALAPPDATA%\Product Mission Control`，並在開始功能表加入捷徑。缺少 WebView2 時安裝程式會一併安裝；Windows 11 已內建。

### 選擇開始的方式

1. 啟動 **Product Mission Control**。第一個畫面問你要怎麼開始。兩個選擇地位相同，之後可以在「設定 → 工作區」切換。

   ![選擇開始的方式：從我的工作區開始，或用範例資料學習](images/zh-TW/s01-first-run.png)

   - **從我的工作區開始**：開啟一個空的工作區，你的紀錄只留在這台電腦上。本手冊第三部說明怎麼設定。
   - **用範例資料學習**：開啟一個供練習的合成工作區。它與你的工作分開，隨時可以重設或刪除。

2. 選 **用範例資料學習**。PMC 會準備範例資料（可能需要一分鐘），然後自動重新啟動。

   ![正在準備範例資料](images/zh-TW/s02-preparing.png)

3. PMC 開在範例工作區的 **Executive Cockpit**。你隨時看得出自己在哪裡：視窗標題結尾標示範例資料（**— 範例資料**；語言跟隨系統時，剛啟動可能先顯示英文 **— Sample data**），上方列顯示 **範例工作區**（點它會開啟「設定 → 工作區」）。

   ![範例工作區：Executive Cockpit，上方列有「範例工作區」標示](images/zh-TW/s03-sample-cockpit.png)

4. 介面語言跟著 Windows。要改的話，開 **Settings**，在 **語言** 選六種語言之一：English、繁體中文、简体中文、日本語、한국어、Español。

   ![Settings：工作區與語言](images/zh-TW/s04-language.png)

---

## 第二部：範例資料的一週

你扮演 Head of Products 過一週。每一步一個動作、一張截圖，依拍攝順序排列。每一個變更都走同一個模式：先**準備**預覽、看清楚核准會做什麼，再**核准**或**拒絕**。拒絕會被記錄；不做決定就關掉預覽，則不會留下任何紀錄。

### 範例資料裡有什麼

| 類別             | 內容                                                                                                                                                               |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Product          | Demo Product Atlas、Beacon、Cinder、Delta                                                                                                                          |
| Stakeholder      | 6 位 `Demo Person: …`，2 個 `Demo Org: …`（Hosting Vendor 為 Confidential）                                                                                        |
| Action Request   | 6 筆，全部待處理。3 筆的回覆期限已過；2 筆沒有負責人                                                                                                               |
| Decision Request | 3 筆，全部待處理                                                                                                                                                   |
| Risk             | 4 筆，全部待處理                                                                                                                                                   |
| Issue            | 4 筆，全部待處理。第 4 筆的標題寫著「second occurrence」，用來呈現重複發生；沒有儲存復發關聯                                                                       |
| Evidence         | `-1` 已驗證、`-2` 讀得到但沒有釘選指紋、`-3` 曾經驗證但目前無法再確認、`-4` 未驗證、`-5` 已驗證（Restricted）、`-6` 已驗證（連在 Action Request 上，不是 Product） |

### 星期一：先看，不動手

這一天不寫入任何東西。

1. PMC 開在 **Executive Cockpit**。Portfolio Lens 依里程碑日期（橫軸）與成果可觀測程度（縱軸）擺放每個 Product；它不排名，也不估算信心。四個範例 Product 都落在「持續觀察」。

   ![Executive Cockpit：四個 Product 都在「持續觀察」](images/zh-TW/01-cockpit.png)

2. 在圖上點 **Demo Product Beacon**。右側列出它的 Lens 指標，下方是 Product 健康檢查：「發生」列出目前需要注意的狀況，每一條都寫著來源紀錄與版本。

   ![選取 Beacon：它的 Lens 指標與「發生」](images/zh-TW/02-cockpit-select.png)

3. 按 Lens 右上的 **證據**。模式只改變標示與說明，不會移動任何 Product。這個模式標示最差證據是「內容不符」或「未驗證」的 Product；此刻沒有被標示的，因為 Beacon 的證據是「讀得到，但沒有釘選指紋」。

   ![證據模式：沒有 Product 被標示，Beacon 的已驗證證據是 0／1](images/zh-TW/03-cockpit-evidence-mode.png)

4. 左側選 **Portfolio**。表格依 Product 名稱排列，畫面明寫這是瀏覽順序，不是優先順序。每一列是同一組 Lens 指標，加上負責人身上的標記與分級；該列最右邊的 **檢視** 會打開同一個健康檢查。

   ![Portfolio：四個 Product 的里程碑、成果可觀測、已驗證證據、象限與負責人身上的標記](images/zh-TW/04-portfolio.png)

5. 左側選 **People**。這裡列出每位 Stakeholder 負責什麼、依賴什麼，以及哪些請求還在等他們回覆。Stakeholder 是你建立的紀錄，不是使用者帳號。

   ![People：誰負責什麼、誰還在等回覆](images/zh-TW/05-people.png)

6. 左側選 **Reviews & Reports**。還沒有核准任何檢視期間，所以沒有比較基準。右邊依 Executive Cockpit 的注意順序列出 Work Queue 標記的 5 件事；按 **到 Work Queue 處置** 前往處理。

   ![Reviews & Reports：還沒有檢視期間；5 件被標記的事](images/zh-TW/06-reviews.png)

7. 左側選 **Product Vault**。Vault 可以使用；表格列出 6 份 Evidence 的驗證狀態、指紋、分級與版本。畫面不會顯示檔案路徑。

   ![Product Vault：6 份 Evidence 的狀態](images/zh-TW/07-vault.png)

8. 左側選 **Settings**。文字縮放只影響這台電腦的畫面，下方的範例會跟著變。**資料來源** 顯示 Ledger 的資料結構版本與目前版本，以及 Vault 是否可以使用。淺色／深色主題用右上角的按鈕切換。

   ![Settings：工作區、語言、文字縮放與資料來源](images/zh-TW/08-settings.png)

### 星期二：處理 Action Request

9. 左側選 **Work Queue**。每一列寫出需要注意什麼、期限、為什麼排在這裡，以及狀態允許的下一步。上方的類型按鈕可以篩選，括號裡是選了之後會看到的筆數。

   ![Work Queue：17 筆，最前面是回覆期限已過的 Action Request](images/zh-TW/09-work-queue.png)

10. 點 **Confirm the Atlas rollout window** 的標題，打開詳情。詳情裡的按鈕和該列相同。

    ![Action Request 詳情：回覆已逾期；下一步是準備接受、婉拒、撤回](images/zh-TW/10-item-detail.png)

11. 按 **準備接受**，審閱單打開。逐項核對：要變更的紀錄與版本、app 配發的新 Action id、主旨、承諾內容、負責人、到期日、核准後會做什麼、分級從哪裡來。

    ![審閱單：接受 Confirm the Atlas rollout window](images/zh-TW/11-review-sheet.png)

12. 往下捲。「核准內容摘要」是完整的 64 字元摘要；「預覽有效至」同時顯示時刻與剩餘分鐘。預覽有效 5 分鐘。

    ![審閱單下半：摘要、準備紀錄與剩餘時間](images/zh-TW/12-review-sheet-digest.png)

13. 按 **核准並接受**。審閱單回報建立的 Action、連結的 Request 與收據編號。

    ![已核准並執行：建立並連結 Action，附收據編號](images/zh-TW/13-approved.png)

    Ledger 留下：Request 的下一個版本、新的 Action、一張只能使用一次的收據，以及稽核紀錄。

14. 按 **關閉**。清單重新讀取：**Action (1)**，左側 Work Queue 的標記數少一。

    ![核准後的 Work Queue：Action (1)](images/zh-TW/14-queue-after-approve.png)

15. 對 **Approve the Beacon pricing experiment** 按 **準備接受**，這次按 **拒絕**。審閱單顯示 **已拒絕。**，核准按鈕不能再按。

    ![已拒絕：核准按鈕停用](images/zh-TW/15-rejected.png)

    拒絕會被記錄：這份預覽被用掉，留下一筆沒有任何效果的稽核紀錄；沒有收據，沒有任何紀錄被改動。

16. 關掉審閱單不等於拒絕。對 **Review the Cinder vendor quote** 按 **準備接受**，然後按 Escape、點審閱單外面，或按 **先不決定**。什麼都不會被記錄。那一列出現 **回到審閱單**，帶你回到同一份預覽；在你決定之前，該列其他按鈕暫停。

    ![先不決定：那一列出現「回到審閱單」](images/zh-TW/16-held-review.png)

17. 預覽放超過 5 分鐘就會過期：核准按鈕停用，仍然可以拒絕，或按 **重新準備** 取得新的預覽。

    ![過期的預覽：可以拒絕或重新準備](images/zh-TW/16b-expired.png)

18. 打開 **Assign an owner for the partner onboarding kit**（沒有負責人），按 **準備接受**。PMC 拒絕執行：缺少必要欄位，重試也不會改變結果。訊息附有可以複製的 Correlation ID。

    ![沒有負責人：準備接受被拒絕，附原因與 Correlation ID](images/zh-TW/17-no-owner-refused.png)

19. 按 **放棄這個動作**，再按 **婉拒**，寫下理由。

    ![婉拒表單：已填入理由](images/zh-TW/18-decline-form.png)

20. 按 **確認婉拒**。清單回報這筆 Request 已婉拒，它從清單消失。

    ![婉拒之後：確認訊息與變短的清單](images/zh-TW/19-declined.png)

    婉拒與撤回都要寫理由；Ledger 會記下理由，並把 Request 推進到下一個版本。

### 星期三：Action 的一生

21. 按 **Action**，找到星期二建立的 Action，按 **開始**。它變成進行中，下一步是 **連結 Evidence**、**準備完成**、**準備取消**。

    ![開始之後：Action 進行中](images/zh-TW/20-action-started.png)

22. 按 **連結 Evidence**，選 `demo-evidence-1`，按 **確認連結**。選單列出每份 Evidence 的驗證狀態、指紋、分級與版本，不顯示檔案路徑。

    ![連結 Evidence：demo-evidence-1（已驗證、已釘選、Internal、版本 1）](images/zh-TW/21-link-evidence.png)

23. 按 **準備完成**。Judgment 理由留白代表不附加；已連結的 Evidence 如果還沒驗證，就一定要寫。

    ![完成表單：Judgment 理由留白](images/zh-TW/22-complete-form.png)

24. 按 **產生完成預覽**。「證據與判斷」寫著證據已足夠，並列出 Evidence 的來源版本與摘要。你核准前如果這份 Evidence 被搬移、重新觀察或釘選，摘要就會不同，核准會被擋下。

    ![完成審閱單：證據已足夠](images/zh-TW/23-complete-sheet.png)

25. 按 **核准並完成**。審閱單回報 Action 已完成。

    ![已核准並執行：Action 已完成](images/zh-TW/24-completed.png)

26. 用同樣方式再接受並開始一筆 Action（第 11–13 步與第 21 步）。連結 `demo-evidence-2`（讀得到但沒有釘選指紋），不寫 Judgment 就按 **準備完成**。PMC 拒絕執行。

    ![準備完成被拒絕：這份證據本身不足以支持](images/zh-TW/25-judgment-required.png)

27. 再按一次 **準備完成**，這次寫下 Judgment 理由並選分級。

    ![完成表單：已填入 Judgment 理由](images/zh-TW/26-judgment-form.png)

28. 按 **產生完成預覽**。這次寫著證據還在等待驗證，先以人工判斷進行，並列出你的判斷。

    ![完成審閱單：以人工判斷進行](images/zh-TW/27-judgment-sheet.png)

29. 按 **核准並完成**。

    ![已核准並執行：以 Judgment 完成](images/zh-TW/28-judgment-completed.png)

30. 開始第三筆 Action，改連結 `demo-evidence-4`（從未驗證）。即使寫了 Judgment，**準備完成** 仍被拒絕。

    ![從未驗證的證據：寫了 Judgment 也無法完成](images/zh-TW/29-no-rescue.png)

    差別在於有沒有東西可以判斷：讀得到但沒釘選、或曾經驗證但現在無法確認的證據，你寫下判斷就可以前進；從未驗證或內容不符的證據，人工判斷也無法替它背書。

31. 按 **放棄這個動作**，再按 **準備取消**，寫下理由、產生預覽，按 **核准並取消**。審閱單回報 Action 已取消。

    ![已核准並執行：Action 已取消](images/zh-TW/30-cancelled.png)

32. 按 **準備重新開啟**，模式維持 **重新啟動已取消的 Action**，寫下理由。

    ![重新開啟表單：模式與理由](images/zh-TW/31-reopen-form.png)

33. 按 **產生重新開啟預覽**，核對內容。

    ![重新開啟審閱單](images/zh-TW/31b-reopen-sheet.png)

34. 按 **核准並重新開啟**。Action 回到進行中。

    ![已核准並執行：Action 回到進行中](images/zh-TW/32-reopened.png)

    完成、取消、重新開啟的預覽都可以拒絕，拒絕一樣會寫進 Ledger。

### 星期四上午：Decision Request

35. 按 **Decision Request**。對 **Choose the Atlas data-residency region** 按 **準備解決**。寫下決定、理由與影響，勾選支持的 Evidence（`demo-evidence-1`），再按 **新增後續 Action Request**，填主旨、說明、負責人、到期日與分級。後續 Request 的 id 由 app 配發。

    ![解決表單：後續 Action Request，負責人 demo-stakeholder-2，到期日 2026-10-31](images/zh-TW/33-decision-form.png)

36. 按 **產生解決預覽**。審閱單列出將建立的 Decision、每一筆後續 Action Request（含 app 配發的 id）、核准後會做什麼、各自的分級來源，以及這個決定依據的 Evidence。

    ![解決審閱單：後續 Request、要變更的紀錄、核准後會做什麼與證據](images/zh-TW/34-decision-sheet.png)

37. 按 **核准並解決**。審閱單回報建立的 Decision、1 筆後續 Action Request 與收據。

    ![已核准並執行：建立 Decision 與 1 筆後續 Action Request](images/zh-TW/35-decision-resolved.png)

    Ledger 在同一個交易裡關閉 Decision Request，並留下 Decision、後續 Request、一張收據與稽核紀錄。

38. 按 **Action Request**。後續 Request **Move the Atlas data store to the EU region** 出現了，可以接受、婉拒或撤回。

    ![Work Queue：新的 Action Request「Move the Atlas data store to the EU region」](images/zh-TW/36-follow-up-request.png)

    它的期限欄顯示 **未設回覆期限**：後續 Request 有承諾完成日（就在下一行），但沒有必須回覆的日期。

### 星期四下午：Risk

39. 按 **Risk**。每筆 Risk 都有 **準備記錄發生**、**準備關閉** 與 **更新應對…**。

    ![Work Queue 只顯示 Risk](images/zh-TW/37-risk-filter.png)

40. 對 **Key vendor concentration** 按 **準備記錄發生**。這個操作沒有要填的欄位。要看的是「將建立的 Issue」：這個 id 由 app 配發，就是核准後會寫進去的那一個。

    ![記錄發生：將建立的 Issue 與核准後會做什麼](images/zh-TW/38-risk-occurrence-sheet.png)

41. 按 **核准並記錄發生**。審閱單回報已記錄發生並建立 Issue。

    ![已核准並執行：已記錄發生並建立 Issue](images/zh-TW/39-risk-occurred.png)

    如果改按拒絕，之後重新準備會得到**另一個** Issue id。**準備關閉** 會要你寫理由，之後一樣是預覽、核准或拒絕。

### 星期四傍晚：Issue

42. 按 **Issue**。5 筆都是待處理，每筆都有 **準備解決**。**Key vendor concentration** 是剛才由 Risk 建立的，沿用 Risk 的標題。**Nightly export fails on large accounts (second occurrence)** 標題裡的「second occurrence」用來呈現重複發生的情境；這個版本不會儲存 Issue 之間的復發關聯。

    ![Work Queue 只顯示 Issue：5 筆待處理](images/zh-TW/40-issue-list.png)

43. 對 **Onboarding email lands in spam** 按 **準備解決**。選解決方式（已解決、以替代方案繞過、接受此影響），寫理由，勾 `demo-evidence-1`。三種解決方式都需要 Evidence。

    ![解決表單：已解決、理由、勾選 demo-evidence-1](images/zh-TW/41-issue-resolve-form.png)

    改勾 `demo-evidence-4`（從未驗證）會被拒絕。勾 `-2` 或 `-3` 則要寫 Judgment 理由並選分級，預覽會說明先以人工判斷進行。

44. 按 **產生解決預覽**，核對內容。

    ![Issue 的解決審閱單](images/zh-TW/41b-issue-resolve-sheet.png)

45. 按 **核准並解決**。這筆 Issue 現在是已解決。

    ![已核准並執行：demo-issue-2 已解決](images/zh-TW/42-issue-resolved.png)

46. 按 **關閉**。這筆 Issue 現在有 **準備關閉**（需要 Evidence：已驗證，或只驗證了一部分而由你寫下的 Judgment 承接）與 **準備重新開啟**（需要能證明先前的解決結果沒有成立的 Evidence，加上你的理由；這份 Evidence 本身仍要通過同一道 Evidence-or-Judgment 規則）。

    ![已解決的 Issue：狀態允許關閉、重新開啟](images/zh-TW/43-issue-next-steps.png)

### 星期五：Evidence

47. 左側選 **Portfolio**，對 **Demo Product Beacon** 按 **檢視**，健康檢查切到 **Evidence** 分頁。`demo-evidence-2` 旁有 **釘選指紋** 與 **重新觀察**；清單下方是 **連結 Evidence 到此 Product** 與 **從檔案新增 Evidence…**。

    ![Beacon 的 Evidence 分頁：demo-evidence-2 讀得到但沒有釘選指紋](images/zh-TW/44-evidence-tab.png)

48. 按 **釘選指紋**。確認文字說明釘選是永久的，並以 id 稱呼這份 Evidence；不顯示檔案路徑。

    ![釘選確認：釘選是永久的](images/zh-TW/45-pin-confirm.png)

49. 按 **確認釘選**。`demo-evidence-2` 變成已驗證；Lens 指標與 Portfolio 表格同時重新讀取，Beacon 變成 1／1 已驗證，「發生」也不再列出這份 Evidence。

    ![釘選之後：demo-evidence-2 已驗證，Beacon 1／1](images/zh-TW/46-pinned.png)

50. 按 **關閉**，對 **Demo Product Atlas** 按 **檢視**，對 `demo-evidence-1` 按 **重新觀察**。PMC 會依這份 Evidence 記錄的位置讀取檔案，只有看到的和已存的不同時才寫入。

    ![重新觀察確認：demo-evidence-1](images/zh-TW/47-reobserve-confirm.png)

51. 按 **確認重新觀察**。結果是已寫入、驗證狀態為已驗證。

    ![重新觀察之後：已寫入，已驗證](images/zh-TW/48-reobserved.png)

    檔案讀得到、內容仍然相符時，每次都會寫入，因為每次觀察都記下新的時間。只有觀察結果與已存狀態完全相同時才顯示「沒有變化」，例如檔案仍然找不到、而且已經這樣記錄。

52. 按 **關閉**，再按 **連結 Evidence 到此 Product**。選單只列出還沒連結到 Atlas 的 Evidence。

    ![連結表單：demo-evidence-2（已驗證、Internal、版本 3）](images/zh-TW/49-link-form.png)

53. 按 **確認連結**。分頁多了 `demo-evidence-2` 與它連結時的分級；Atlas 在健康檢查與 Portfolio 表格都變成 2／2 已驗證。

    ![連結之後：Atlas 有兩份已驗證的 Evidence](images/zh-TW/50-linked.png)

### 同樣在星期五：新增紀錄

54. 紀錄在它所屬的畫面建立。在 **Portfolio** 按 **新增 Product…**，寫名稱與說明，選分級。

    ![新增 Product：名稱、說明與分級](images/zh-TW/51-new-product.png)

55. 按 **建立**。頁面回報新的 Product 與它的 id，表格變成 5 個 Product。

    ![已建立新的 Product](images/zh-TW/51b-product-created.png)

    Portfolio、Initiative、Project、Milestone、Roadmap、KPI、Stakeholder、Action Request、Decision Request、Risk、Issue 都用同樣方式，各自在自己的畫面新增。

56. Evidence 從 Vault 資料夾裡的檔案開始。在 Product 的 **Evidence** 分頁按 **從檔案新增 Evidence…**，再按 **選擇檔案…**。Windows 會開啟它的選擇檔案視窗：選 Vault 資料夾裡的檔案（資料夾外的檔案會被拒絕），按 **開啟**。PMC 顯示檔名與觀察時間；選擇分級。

    ![從檔案新增 Evidence：選好的檔案、觀察時間與分級](images/zh-TW/52-evidence-from-file.png)

57. 按 **建立並連結到這個 Product**。PMC 記下檔案的位置與指紋；檔案留在原處，不會被複製。

    ![已建立新的 Evidence，並連結到 Demo Product Atlas](images/zh-TW/52b-evidence-created.png)

### 其他你應該知道的

- **每一個錯誤** 都附可以複製的 Correlation ID。可以重試的錯誤按「重試」時會沿用同一個請求 id，所以重試不會變成第二筆寫入。
- **每一個受管變更** 都可以在核准前退出：「拒絕」會被記錄；Escape、點審閱單外面或「先不決定」只是關掉，之後可以從「回到審閱單」回到同一份預覽。
- **Vault 無法使用時**，釘選與重新觀察會停用並說明原因；連結不受影響。
- **搬移** Evidence 檔案到新位置，目前還不能在畫面上做。

---

## 第三部：開始使用自己的工作區

你的工作區存放真正的紀錄。它要在一份備份驗證通過之後才接受新增紀錄，所以設定順序很重要。完成之前，Executive Cockpit 上方的 **開始使用** 清單會告訴你做到哪裡。

1. 在 **Settings → 工作區** 按 **切換到我的工作區**，再按 **切換並重新啟動**。PMC 會在你的工作區重新啟動。（在首次啟動畫面選 **從我的工作區開始** 也一樣。）

   ![切換到我的工作區：PMC 會重新啟動](images/zh-TW/p01-switch-confirm.png)

2. 你的工作區一開始是空的。**備份已到期** 提示列說明完成備份後才能再新增或修改紀錄；**開始使用** 依序列出五個設定步驟。每一步都依 PMC 實際看得到的狀態判斷，只有下一步有按鈕。

   ![空的工作區：備份已到期提示列與開始使用清單](images/zh-TW/p02-live-empty.png)

3. 按 **開啟設定**。在 **備份** 按 **選擇資料夾…**。Windows 會開啟選擇資料夾視窗：選一個資料夾（最好在另一顆磁碟上），按 **選擇資料夾**。畫面顯示 **已設定** 與 **可以使用**。

   ![備份：備份資料夾已設定](images/zh-TW/p03-backup-folder-set.png)

4. 按 **設定密語…**。PMC 產生一組密語，而且**只顯示這一次**。把它寫下來或存進密碼管理工具，再在 **完整輸入一次** 打一遍。（按 **改用我自己的密語** 可以自己設定。）勾選你了解 PMC 無法找回密語。

   ![設定復原密語：只顯示一次](images/zh-TW/p04-passphrase.png)

   勾 **記在這個 Windows 帳戶，讓 PMC 自動備份。** 會把密語存進 Windows 認證管理員，備份就能自動進行；不勾的話，密語只保留到 PMC 關閉為止。按 **使用這組密語**。

   ![已再輸入一次密語，並勾選確認](images/zh-TW/p04b-passphrase-typed.png)

   備份用這組密語加密。**沒有它，任何一台電腦都無法還原這份備份，而且 PMC 無法找回遺失的密語。**

5. 按 **立即備份**。備份寫入並讀回驗證後，「最近一次備份」會顯示驗證時間與下一次到期時間，備份已到期提示列也會消失。

   ![驗證通過的備份，下一次在一天後到期](images/zh-TW/p05-backup-verified.png)

   備份包含 Ledger 與 PMC 的非機密設定，不包含 Vault 或 Evidence 檔案；那個資料夾請另外備份。

6. 在 Settings 往下捲，到 **資料來源 → Product Vault** 按 **選擇 Vault 資料夾…**。PMC 會說明變更前會先備份這個工作區；再按一次 **選擇 Vault 資料夾…**，Windows 會開啟選擇資料夾視窗：選存放（或將要存放）Evidence 檔案的資料夾。接著 PMC 會列出確切的變更：目前與新的資料夾、受影響的 Evidence，以及剛做好的備份。輸入畫面上的確認短語，按 **使用這個資料夾**。

   ![變更 Vault 資料夾：會變更什麼，以及事先做好的備份](images/zh-TW/p06-vault-confirm.png)

   Vault 現在顯示 **可以使用**，按鈕變成 **更換 Vault 資料夾…**。

   ![Vault 資料夾已設定](images/zh-TW/p07-vault-set.png)

7. 回到 Executive Cockpit，開始使用清單顯示前四步已完成，下一步是 **新增第一個 Product**。按 **開啟 Portfolio**，建立第一批紀錄：**新增 Portfolio…**，再 **新增 Product…**。

   ![你的第一個 Portfolio 與 Product](images/zh-TW/p08-first-product.png)

   第一個 Product 建立後，五個步驟都完成，開始使用清單就會消失；Product 會出現在 Portfolio Lens 上。

   ![設定完成後的 Executive Cockpit：開始使用清單已消失](images/zh-TW/p08b-cockpit-complete.png)

8. 把一個檔案放進 Vault 資料夾，打開 Product 的 **Evidence** 分頁，照第二部第 56 步用 **從檔案新增 Evidence…**。

   ![用 Vault 裡的檔案建立 Evidence](images/zh-TW/p09-evidence-created.png)

9. 要還原時，在「Settings → 備份」按 **從備份還原…**，再按 **選擇備份檔…**。Windows 會開啟選擇檔案視窗：從備份資料夾（或另一台電腦的備份）選一個 Operational Backup（`.tar.zst.age` 檔）。輸入 **這份備份的密語**，按 **檢查備份**。

   ![還原：選好的備份與它的密語](images/zh-TW/p10a-restore-passphrase.png)

   顯示任何內容之前，PMC 會先備份目前的工作區。接著預覽會清楚列出還原會做什麼：備份的建立時間與紀錄數、目前的狀態、**會被取代** 的內容（Product Ledger 與備份裡的設定）、**不會被取代** 的內容（Product Vault、備份資料夾與密語設定、其他所有備份），以及剛做好的 **復原用備份**。要繼續，輸入備份的建立日期並按 **以這份備份取代**；按 **不還原** 則目前的工作區不會改變，剛做好的復原用備份會保留。

   ![還原預覽](images/zh-TW/p10-restore-preview.png)

10. 你在自己的工作區時，範例資料仍然留著。在 **Settings → 工作區** 可以 **重設範例資料…**（回到初始狀態），或 **刪除範例資料…**：它會列出確切移除的內容，並要你輸入「刪除範例資料」。兩者都不會影響你的工作區、設定與備份。

    ![重設範例資料：確認](images/zh-TW/p11-reset-confirm.png)

    ![刪除範例資料：確切移除的內容](images/zh-TW/p12-delete-sheet.png)

11. 左側最下方的 **System Health** 告訴你 PMC 能否讀取並保存你的紀錄。

    ![System Health：Product Ledger 已開啟、可以讀取](images/zh-TW/p13-system-health.png)

---

## 第四部：解除安裝

在 **Windows 設定 → 應用程式 → 已安裝的應用程式 → Product Mission Control → 解除安裝**。

解除安裝會移除程式與開始功能表捷徑，**不會刪除你的資料**。以下都會留在原處：

| 內容                         | 位置                                               |
| ---------------------------- | -------------------------------------------------- |
| 你的紀錄（Ledger）與範例資料 | `%LOCALAPPDATA%\ProductMissionControlDesktop`      |
| app 的 WebView2 資料         | `%LOCALAPPDATA%\com.productmissioncontrol.desktop` |
| 你的備份                     | 你選的備份資料夾                                   |
| 你的 Evidence 檔案           | 你選的 Vault 資料夾                                |

之後重新安裝，會開啟同一份紀錄。若也要移除資料，請在解除安裝後自行刪除上述資料夾；如果日後可能還需要這些紀錄，先保留一份備份與它的密語。

import type { ROUTES_EN } from "./routes.en";

/** 各頁面的繁體中文文字。鍵與英文完全相同；用字沿用原本畫面上的文字。 */
export const ROUTES_ZH_TW = {
  "route.loading": "正在讀取 {route}。",
  "route.reload": "重新讀取",
  "route.unavailable": "{route} 目前無法讀取。畫面沒有顯示任何舊資料。 {message}",
  "route.outOfSync": "讀取途中 Ledger 有更新，兩次讀到的版本不一致，所以這次沒有顯示任何內容。",
  "route.ledgerRevision": "Ledger 版本",
  "route.readAt": "讀取於",

  "cockpit.headline": "早安，今天先處理真正影響結果的事。",
  "cockpit.lede": "以里程碑日期、成果可觀測程度與已驗證證據看每個 Product；不排名，也不估算信心。",
  "cockpit.lensModes": "標示方式",
  "cockpit.lensEmpty":
    "Ledger 裡還沒有任何 Product。建立 Product 之後，這裡會依里程碑、KPI 與證據擺放它們。",
  "cockpit.lensLegend":
    "圓越大，已驗證的證據占比越高；虛線圓代表沒有連結的證據。只有兩個軸都有資料時才歸入象限。",
  "cockpit.asideEmpty":
    "在圖上或表格裡選一個 Product，這裡會列出它的指標、背後的紀錄與目前的狀態。",
  "cockpit.period": "期間比較",
  "cockpit.periodUnavailable": "無法比較：{reason}",
  "cockpit.pulse": "Portfolio 現況",
  "cockpit.pulse.milestones": "Milestones",
  "cockpit.pulse.commitments": "Commitments",
  "cockpit.pulse.kpis": "KPIs",
  "cockpit.pulse.from": "來自{owner}",
  "cockpit.attention": "需要注意的事項",
  "cockpit.attentionNone": "目前沒有需要注意的事項。",
  "cockpit.placedBecause": "排在這裡：{tier}",
  "cockpit.briefing": "給你的摘要",
  "cockpit.briefingNone": "目前整個 Portfolio 沒有需要你注意的事項。",
  "cockpit.briefingTop.one": "有 {count} 件事需要注意。最前面的是「{label}」：{reason}。",
  "cockpit.briefingTop.other": "有 {count} 件事需要注意。最前面的是「{label}」：{reason}。",
  "cockpit.briefingWhy": "它排在最前面，因為{tier}。",
  "cockpit.briefingNoPeriod": "目前沒有可比較的期間，所以看不出這些事是變好還是變差。",

  "portfolio.headline": "Product 組合一覽",
  "portfolio.lede":
    "每個 Product 的里程碑、成果可觀測程度與已驗證證據，以及負責人身上被標記的工作。",
  "portfolio.showing": "共 {total} 個，目前顯示第 {from} 到 {to} 個。",
  "portfolio.empty": "Ledger 裡還沒有任何 Product。建立 Product 之後，這裡會列出它們。",
  "portfolio.emptyPage": "這一頁沒有 Product；共有 {total} 個。",
  "portfolio.firstPage": "回到第一頁",
  "portfolio.nextPage": "顯示下一頁",
  "portfolio.column.flagged": "負責人身上的標記",
  "portfolio.column.classification": "分級",
  "portfolio.flaggedNone": "沒有",
  "portfolio.flaggedCount": "{count} 件",
  "portfolio.asideEmpty": "在表格裡選一個 Product，這裡會列出它的指標、背後的紀錄與目前的狀態。",
  "productAside.label": "選取的 Product",
} as const satisfies Record<keyof typeof ROUTES_EN, string>;

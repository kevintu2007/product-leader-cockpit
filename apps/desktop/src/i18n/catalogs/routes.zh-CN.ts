import type { ROUTES_EN } from "./routes.en";

/** 各页面的简体中文文字。键与英文完全相同；由 zh-TW 目录转写，采用大陆用语。 */
export const ROUTES_ZH_CN = {
  "route.loading": "正在读取 {route}。",
  "route.reload": "重新读取",
  "route.unavailable": "{route} 目前无法读取。界面没有显示任何旧数据。 {message}",
  "route.outOfSync": "读取途中 Ledger 有更新，两次读到的版本不一致，所以这次没有显示任何内容。",
  "route.ledgerRevision": "Ledger 版本",
  "route.readAt": "读取于",

  "cockpit.headline": "早上好，今天先处理真正影响结果的事。",
  "cockpit.lede": "以里程碑日期、成果可观测程度与已验证证据看每个 Product；不排名，也不估算信心。",
  "cockpit.lensModes": "标记方式",
  "cockpit.lensEmpty":
    "Ledger 里还没有任何 Product。创建 Product 之后，这里会依里程碑、KPI 与证据摆放它们。",
  "cockpit.lensLegend":
    "圆越大，已验证的证据占比越高；虚线圆代表没有关联的证据。只有两个轴都有数据时才归入象限。",
  "cockpit.asideEmpty":
    "在图上或表格里选一个 Product，这里会列出它的指标、背后的记录与目前的状态。",
  "cockpit.period": "期间比较",
  "cockpit.periodUnavailable": "无法比较：{reason}",
  "cockpit.pulse": "Portfolio 现状",
  "cockpit.pulse.milestones": "Milestones",
  "cockpit.pulse.commitments": "Commitments",
  "cockpit.pulse.kpis": "KPIs",
  "cockpit.pulse.from": "来自{owner}",
  "cockpit.attention": "需要注意的事项",
  "cockpit.attentionNone": "目前没有需要注意的事项。",
  "cockpit.placedBecause": "排在这里：{tier}",
  "cockpit.briefing": "给你的摘要",
  "cockpit.briefingNone": "目前整个 Portfolio 没有需要你注意的事项。",
  "cockpit.briefingTop.one": "有 {count} 件事需要注意。最前面的是「{label}」：{reason}。",
  "cockpit.briefingTop.other": "有 {count} 件事需要注意。最前面的是「{label}」：{reason}。",
  "cockpit.briefingWhy": "它排在最前面，因为{tier}。",
  "cockpit.briefingNoPeriod": "目前没有可比较的期间，所以看不出这些事是变好还是变差。",

  "portfolio.headline": "Product 组合一览",
  "portfolio.lede":
    "每个 Product 的里程碑、成果可观测程度与已验证证据，以及负责人身上被标记的工作。",
  "portfolio.showing": "共 {total} 个，目前显示第 {from} 到 {to} 个。",
  "portfolio.empty": "Ledger 里还没有任何 Product。创建 Product 之后，这里会列出它们。",
  "portfolio.emptyPage": "这一页没有 Product；共有 {total} 个。",
  "portfolio.firstPage": "回到第一页",
  "portfolio.nextPage": "显示下一页",
  "portfolio.column.flagged": "负责人身上的标记",
  "portfolio.column.classification": "分级",
  "portfolio.flaggedNone": "没有",
  "portfolio.flaggedCount": "{count} 件",
  "portfolio.asideEmpty": "在表格里选一个 Product，这里会列出它的指标、背后的记录与目前的状态。",
  "productAside.label": "选中的 Product",
} as const satisfies Record<keyof typeof ROUTES_EN, string>;

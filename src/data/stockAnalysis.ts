export type StockAnalysisSnapshot = {
  ticker: string
  name: string
  industry: string
  listedDate: string
  dataAt: string
  mode: string
  quote: {
    price: number
    prevClose: number
    changePct: number
    marketCap: string
    peTtm: number
    peValuation: string
    pb: string
  }
  financial: {
    years: readonly string[]
    revenue: readonly number[]
    netProfit: readonly number[]
    roe: readonly number[]
    period: string
    revenueGrowthYoy: number
    revenueGrowthPeriod: string
    revenueGrowthLabel: string
    netProfitYoyPct: number
    health: {
      currentRatio: number
      debtRatio: number
      netMarginPct: number
    }
    dupont: {
      netMarginPct: number
      assetTurnover: number
      equityMultiplier: number
      roeReconstructedPct: number
      roeQuality: string
    }
  }
  technical: {
    stage: string
    maAlignment: string
    ma20: number
    ma200: number
    aboveMa20: boolean
    aboveMa200: boolean
    rsi14: number
    ytdReturn: string
    yearHigh: number
    yearLow: number
    pctFromYearHigh: number
  }
  valuation: {
    dcf: {
      intrinsicPerShare: number
      currentPrice: number
      safetyMarginPct: number
      wacc: number
      terminalGrowth: number
      terminalValuePctOfEv: number
      stage1Growth: number
      stage2Growth: number
      sensitivity: readonly {
        wacc: string
        terminalGrowth: string
        value: number
      }[]
    }
    peerCount: number
    peQuantile: string
    pbQuantile: string
  }
  events: readonly {
    date: string
    title: string
  }[]
  integrity: {
    coveragePct: number
    passedChecks: number
    totalChecks: number
    criticalMissing: boolean
    missingDimensions: readonly string[]
    fallbackDimensions: readonly string[]
  }
}

/**
 * 600519.SH stage1 只读快照。
 *
 * 数字逐项取自 stock-deep-analyzer:uzi 的本机缓存；这里只固化为界面展示数据，
 * 不在前端重新估算、换算或补齐缺失字段。
 */
export const stockAnalysis600519: StockAnalysisSnapshot = {
  ticker: '600519.SH',
  name: '贵州茅台',
  industry: '白酒',
  listedDate: '2001-08-27',
  dataAt: '2026-09-11T09:54:39',
  mode: 'Stage 1 只读快照',
  quote: {
    price: 1269.44,
    prevClose: 1285.13,
    changePct: -1.22,
    marketCap: '15869.0亿',
    peTtm: 17.82,
    peValuation: '19.51',
    pb: '6.32',
  },
  financial: {
    years: ['2020', '2021', '2022', '2023', '2024', '2025'],
    revenue: [979.93, 1094.64, 1275.54, 1505.6, 1741.44, 1720.54],
    netProfit: [466.97, 524.6, 627.17, 747.34, 862.28, 823.2],
    roe: [31.41, 29.9, 30.26, 34.19, 36.02, 32.53],
    period: '2025-12-31',
    revenueGrowthYoy: 1.47,
    revenueGrowthPeriod: '2026-06-30',
    revenueGrowthLabel: '+1.5%',
    netProfitYoyPct: -4.5,
    health: {
      currentRatio: 5.5895,
      debtRatio: 15.1931,
      netMarginPct: 50.5279,
    },
    dupont: {
      netMarginPct: 50.53,
      assetTurnover: 0.56,
      equityMultiplier: 1.2,
      roeReconstructedPct: 33.86,
      roeQuality: 'margin_driven',
    },
  },
  technical: {
    stage: 'Stage 4 下跌',
    maAlignment: '非多头',
    ma20: 1298.0335,
    ma200: 1340.1818499999997,
    aboveMa20: false,
    aboveMa200: false,
    rsi14: 34.65225727732263,
    ytdReturn: '-11.1%',
    yearHigh: 1526.98,
    yearLow: 1168.63,
    pctFromYearHigh: -16.865970739629855,
  },
  valuation: {
    dcf: {
      intrinsicPerShare: 1842.99,
      currentPrice: 1269.44,
      safetyMarginPct: 45.2,
      wacc: 0.0696,
      terminalGrowth: 0.025,
      terminalValuePctOfEv: 68.9,
      stage1Growth: 0.1,
      stage2Growth: 0.05,
      sensitivity: [
        { wacc: '7.0%', terminalGrowth: '1.5%', value: 1600.31 },
        { wacc: '7.0%', terminalGrowth: '2.5%', value: 1842.99 },
        { wacc: '7.0%', terminalGrowth: '3.5%', value: 2225.96 },
      ],
    },
    peerCount: 0,
    peQuantile: '未提供',
    pbQuantile: '未提供',
  },
  events: [
    {
      date: '2026-09-08',
      title: '茅台年内第六次调价：自营店飞天涨至1766元；工作人员称“门店产品充足”',
    },
    {
      date: '2026-09-09',
      title: 'i茅台次新飞天开启常态化售卖，由每月仅三天到每日两场可购',
    },
    {
      date: '2026-08-15',
      title: '2026 年中报净利润为445.17亿元、同比较去年同期下降1.95%',
    },
  ],
  integrity: {
    coveragePct: 83.0,
    passedChecks: 15,
    totalChecks: 18,
    criticalMissing: false,
    missingDimensions: [
      'PE/PB 历史分位',
      '护城河评分',
      '同业对标',
      '产业链',
      '券商研报',
      '资金流',
      '舆情',
    ],
    fallbackDimensions: ['宏观', '行业', '原材料', '期货', '政策'],
  },
}

<template>
  <article class="panel history-panel">
    <div class="panel-head">
      <h2><em class="green-dot" /> 余额 / 机会历史</h2>
      <span>UTC / 会话</span>
    </div>
    <div class="chart-wrap">
      <svg viewBox="0 0 640 210" preserveAspectRatio="none" aria-label="history chart">
        <defs>
          <linearGradient id="fill" x1="0" x2="0" y1="0" y2="1">
            <stop offset="0" stop-color="#9adbc4" stop-opacity=".42" />
            <stop offset="1" stop-color="#9adbc4" stop-opacity="0" />
          </linearGradient>
        </defs>
        <path d="M0 162 C70 160 95 166 155 151 S245 158 296 144 S354 149 400 142 S455 150 492 136 S526 141 552 75 S590 106 640 43 L640 210 L0 210Z" fill="url(#fill)" />
        <path d="M0 162 C70 160 95 166 155 151 S245 158 296 144 S354 149 400 142 S455 150 492 136 S526 141 552 75 S590 106 640 43" fill="none" stroke="#28352f" stroke-width="3" />
        <circle cx="552" cy="75" r="6" fill="#dc5974" />
        <line x1="552" y1="75" x2="552" y2="210" stroke="#dc5974" stroke-dasharray="3 5" />
      </svg>
      <div class="axis">
        <span>开始</span>
        <span>观测窗口</span>
        <span>现在</span>
      </div>
    </div>
  </article>
</template>

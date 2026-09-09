<script setup lang="ts">
import { ref, onMounted, reactive } from 'vue'
import { ElButton, ElInput, ElInputNumber, ElMessage, ElSwitch } from 'element-plus'
import {
  DEFAULT_APP_CONFIG,
  DEFAULT_OBSERVER_CONFIG,
  invokeCommand,
  saveObserverConfig as saveObserverConfigIpc,
  COMMANDS,
} from '../commands'
import type { AppConfig, ObserverConfig, PairConfig } from '../commands'
import TermHint from './TermHint.vue'

/** 常见现货交易对预设：添加币种时下拉选择，选中后自动填充四字段（仍可手动修改）。 */
const PRESET_PAIRS: PairConfig[] = [
  { symbol: 'BTCUSDT', base_asset: 'BTC', quote_asset: 'USDT', quantity: '0.001' },
  { symbol: 'ETHUSDT', base_asset: 'ETH', quote_asset: 'USDT', quantity: '0.01' },
  { symbol: 'SOLUSDT', base_asset: 'SOL', quote_asset: 'USDT', quantity: '1' },
  { symbol: 'BNBUSDT', base_asset: 'BNB', quote_asset: 'USDT', quantity: '0.01' },
  { symbol: 'XRPUSDT', base_asset: 'XRP', quote_asset: 'USDT', quantity: '20' },
  { symbol: 'DOGEUSDT', base_asset: 'DOGE', quote_asset: 'USDT', quantity: '100' },
  { symbol: 'ADAUSDT', base_asset: 'ADA', quote_asset: 'USDT', quantity: '20' },
  { symbol: 'AVAXUSDT', base_asset: 'AVAX', quote_asset: 'USDT', quantity: '0.1' },
  { symbol: 'LINKUSDT', base_asset: 'LINK', quote_asset: 'USDT', quantity: '1' },
  { symbol: 'DOTUSDT', base_asset: 'DOT', quote_asset: 'USDT', quantity: '1' },
]

const props = defineProps<{
  canOperate: boolean
}>()

const emit = defineEmits<{
  configSaved: [config: AppConfig]
  observerSaved: []
}>()

const loading = ref(false)
const savingEnv = ref(false)
const savingObserver = ref(false)

const envConfig = reactive<AppConfig>({ ...DEFAULT_APP_CONFIG })

const observerConfig = reactive<ObserverConfig>(
  JSON.parse(JSON.stringify(DEFAULT_OBSERVER_CONFIG)),
)

async function loadConfig() {
  loading.value = true
  try {
    const [envData, observerData] = await Promise.all([
      invokeCommand<AppConfig>(COMMANDS.loadAppConfig),
      invokeCommand<ObserverConfig>(COMMANDS.getObserverConfig, { configPath: null }),
    ])
    if (envData !== undefined) {
      Object.assign(envConfig, envData)
    }
    if (observerData !== undefined) {
      // 后端总是返回完整对象：直接覆盖默认模板；pairs 缺数组时保留默认模板避免模板崩溃
      if (Array.isArray(observerData.pairs)) {
        Object.assign(observerConfig, observerData)
      } else {
        Object.assign(observerConfig, { ...observerData, pairs: observerConfig.pairs })
      }
    }
  } finally {
    loading.value = false
  }
}

async function saveEnvConfig() {
  savingEnv.value = true
  try {
    // 成功时 data 为 null（后端 ApiResponse<()>），以 undefined 区分失败
    const saved = await invokeCommand<null>(COMMANDS.saveAppConfig, { config: { ...envConfig } })
    if (saved !== undefined) {
      ElMessage.success('凭据已保存；数据库连接即刻生效，交易所密钥等其余配置重启后生效')
      emit('configSaved', { ...envConfig })
    }
  } finally {
    savingEnv.value = false
  }
}

async function saveObserver() {
  savingObserver.value = true
  try {
    // 观察配置走 json 优先加载链：保存后即时生效，无需重启
    const saved = await saveObserverConfigIpc(JSON.parse(JSON.stringify(observerConfig)))
    if (saved) {
      ElMessage.success('观察配置已保存并即时生效')
      emit('observerSaved')
    }
  } finally {
    savingObserver.value = false
  }
}

/** 下拉选中预设交易对即添加；已在列表中的 symbol 在选项中禁用。 */
const selectedPairSymbol = ref<string>()

function addPair(presetSymbol: string) {
  const preset = PRESET_PAIRS.find(p => p.symbol === presetSymbol)
  if (!preset) return
  observerConfig.pairs.push({ ...preset })
  selectedPairSymbol.value = ''
}

function removePair(idx: number) {
  if (observerConfig.pairs.length <= 1) return
  observerConfig.pairs.splice(idx, 1)
}

onMounted(() => {
  loadConfig()
})
</script>

<template>
  <section class="settings-page">
    <div class="settings-header">
      <h2>系统设置</h2>
      <small>后端配置全部由前端维护：填写默认值或修改后保存为 JSON，后端启动时读取</small>
    </div>

    <div class="settings-section">
      <h3>数据库与交易所凭据</h3>
      <div class="settings-grid">
        <!-- 数据库 -->
        <div class="settings-card">
          <div class="card-header">
            <span class="card-title">数据库</span>
          </div>
          <div class="card-body">
            <div class="field-group">
              <label><TermHint term="database_url" /></label>
              <ElInput v-model="envConfig.database_url" size="small" placeholder="postgresql://user:pass@host:port/db" />
              <small>PostgreSQL 连接地址，用于 PAPER 和账务验证</small>
            </div>
          </div>
        </div>

        <!-- Binance -->
        <div class="settings-card">
          <div class="card-header">
            <span class="card-title">Binance API</span>
          </div>
          <div class="card-body">
            <div class="field-group">
              <label><TermHint term="binance_api_key" /></label>
              <ElInput v-model="envConfig.binance_api_key" size="small" show-password placeholder="Binance API Key" />
            </div>
            <div class="field-group">
              <label><TermHint term="binance_api_secret" /></label>
              <ElInput v-model="envConfig.binance_api_secret" size="small" show-password placeholder="Binance API Secret" />
            </div>
          </div>
        </div>

        <!-- Bybit -->
        <div class="settings-card">
          <div class="card-header">
            <span class="card-title">Bybit API</span>
          </div>
          <div class="card-body">
            <div class="field-group">
              <label><TermHint term="bybit_api_key" /></label>
              <ElInput v-model="envConfig.bybit_api_key" size="small" show-password placeholder="Bybit API Key" />
            </div>
            <div class="field-group">
              <label><TermHint term="bybit_api_secret" /></label>
              <ElInput v-model="envConfig.bybit_api_secret" size="small" show-password placeholder="Bybit API Secret" />
            </div>
          </div>
        </div>
      </div>
    </div>

    <div class="settings-section">
      <h3>观察配置（ObserverConfig）</h3>
      <div class="settings-grid">
        <!-- 交易对列表 -->
        <div class="settings-card">
          <div class="card-header">
            <span class="card-title">交易对列表（pairs）</span>
            <ElSelect
              v-model="selectedPairSymbol"
              size="small"
              :disabled="!canOperate"
              placeholder="添加币种…"
              class="pair-add-select"
              @change="addPair"
            >
              <ElOption
                v-for="preset in PRESET_PAIRS"
                :key="preset.symbol"
                :label="`${preset.symbol}（${preset.quantity} ${preset.base_asset}）`"
                :value="preset.symbol"
                :disabled="observerConfig.pairs.some(p => p.symbol === preset.symbol)"
              />
            </ElSelect>
          </div>
          <div class="card-body">
            <div
              v-for="(pair, idx) in observerConfig.pairs"
              :key="idx"
              class="pair-editor"
            >
              <div class="pair-editor-head">
                <b>{{ pair.symbol || `币种 ${idx + 1}` }}</b>
                <ElButton
                  size="small"
                  text
                  type="danger"
                  :disabled="observerConfig.pairs.length <= 1 || !canOperate"
                  @click="removePair(idx)"
                >删除</ElButton>
              </div>
              <div class="pair-editor-grid">
                <div class="field-group">
                  <label>symbol</label>
                  <ElInput v-model="pair.symbol" size="small" />
                </div>
                <div class="field-group">
                  <label><TermHint term="base_asset" /></label>
                  <ElInput v-model="pair.base_asset" size="small" />
                </div>
                <div class="field-group">
                  <label><TermHint term="quote_asset" /></label>
                  <ElInput v-model="pair.quote_asset" size="small" />
                </div>
                <div class="field-group">
                  <label><TermHint term="quantity" />（数量，小数文本）</label>
                  <ElInput v-model="pair.quantity" size="small" />
                </div>
              </div>
            </div>
            <div class="field-group" style="margin-top: 12px;">
              <label><TermHint term="orderbook_depth" />（档位）</label>
              <ElInputNumber v-model="observerConfig.orderbook_depth" size="small" :min="1" :max="100" controls-position="right" />
            </div>
          </div>
        </div>

        <!-- 时序参数 -->
        <div class="settings-card">
          <div class="card-header">
            <span class="card-title">时序参数（毫秒）</span>
          </div>
          <div class="card-body">
            <div class="field-group">
              <label><TermHint term="poll_interval_ms" />（轮询间隔）</label>
              <ElInputNumber v-model="observerConfig.poll_interval_ms" size="small" :min="0" controls-position="right" />
            </div>
            <div class="field-group">
              <label><TermHint term="http_timeout_ms" />（HTTP 超时）</label>
              <ElInputNumber v-model="observerConfig.http_timeout_ms" size="small" :min="0" controls-position="right" />
            </div>
            <div class="field-group">
              <label><TermHint term="stream_start_timeout_ms" />（流启动超时）</label>
              <ElInputNumber v-model="observerConfig.stream_start_timeout_ms" size="small" :min="0" controls-position="right" />
            </div>
            <div class="field-group">
              <label><TermHint term="reconnect_delay_ms" />（重连延迟）</label>
              <ElInputNumber v-model="observerConfig.reconnect_delay_ms" size="small" :min="0" controls-position="right" />
            </div>
            <div class="field-group">
              <label><TermHint term="max_snapshot_age_ms" />（快照最大年龄）</label>
              <ElInputNumber v-model="observerConfig.max_snapshot_age_ms" size="small" :min="0" controls-position="right" />
            </div>
            <div class="field-group">
              <label><TermHint term="max_pair_skew_ms" />（价差最大年龄）</label>
              <ElInputNumber v-model="observerConfig.max_pair_skew_ms" size="small" :min="0" controls-position="right" />
            </div>
            <div class="field-group">
              <label><TermHint term="account_refresh_interval_ms" />（账户刷新间隔）</label>
              <ElInputNumber v-model="observerConfig.account_refresh_interval_ms" size="small" :min="0" controls-position="right" />
            </div>
            <div class="field-group">
              <label><TermHint term="max_fee_age_ms" />（费率最大年龄）</label>
              <ElInputNumber v-model="observerConfig.max_fee_age_ms" size="small" :min="0" controls-position="right" />
            </div>
            <div class="field-group">
              <label><TermHint term="auth_recv_window_ms" />（签名窗口）</label>
              <ElInputNumber v-model="observerConfig.auth_recv_window_ms" size="small" :min="0" controls-position="right" />
            </div>
          </div>
        </div>

        <!-- 归档 -->
        <div class="settings-card">
          <div class="card-header">
            <span class="card-title">归档（archive）</span>
          </div>
          <div class="card-body">
            <div class="field-group">
              <label>path</label>
              <ElInput v-model="observerConfig.archive.path" size="small" />
            </div>
            <div class="field-group">
              <label><TermHint term="queue_capacity" />（队列容量）</label>
              <ElInputNumber v-model="observerConfig.archive.queue_capacity" size="small" :min="1" controls-position="right" />
            </div>
            <div class="field-group">
              <label><TermHint term="raw_retention_days" />（原始保留天数）</label>
              <ElInputNumber v-model="observerConfig.archive.raw_retention_days" size="small" :min="0" controls-position="right" />
            </div>
          </div>
        </div>

        <!-- Binance 场所 -->
        <div class="settings-card">
          <div class="card-header">
            <span class="card-title">Binance 场所</span>
          </div>
          <div class="card-body">
            <div class="field-group">
              <label>base_url</label>
              <ElInput v-model="observerConfig.binance.base_url" size="small" />
            </div>
            <div class="field-group">
              <label>websocket_url</label>
              <ElInput v-model="observerConfig.binance.websocket_url" size="small" />
            </div>
            <div class="field-group">
              <label><TermHint term="fallback_taker_fee_rate" />（备用吃单费率，小数文本）</label>
              <ElInput v-model="observerConfig.binance.fallback_taker_fee_rate" size="small" />
            </div>
            <div class="field-group">
              <label><TermHint term="api_key_env" />（密钥环境变量名）</label>
              <ElInput v-model="observerConfig.binance.api_key_env" size="small" />
            </div>
            <div class="field-group">
              <label><TermHint term="api_secret_env" /></label>
              <ElInput v-model="observerConfig.binance.api_secret_env" size="small" />
            </div>
            <div class="field-group">
              <label><TermHint term="region_eligible_confirmed" /></label>
              <ElSwitch v-model="observerConfig.binance.region_eligible_confirmed" />
            </div>
            <div class="field-group">
              <label><TermHint term="account_eligible_confirmed" /></label>
              <ElSwitch v-model="observerConfig.binance.account_eligible_confirmed" />
            </div>
          </div>
        </div>

        <!-- Bybit 场所 -->
        <div class="settings-card">
          <div class="card-header">
            <span class="card-title">Bybit 场所</span>
          </div>
          <div class="card-body">
            <div class="field-group">
              <label>base_url</label>
              <ElInput v-model="observerConfig.bybit.base_url" size="small" />
            </div>
            <div class="field-group">
              <label>websocket_url</label>
              <ElInput v-model="observerConfig.bybit.websocket_url" size="small" />
            </div>
            <div class="field-group">
              <label><TermHint term="fallback_taker_fee_rate" />（备用吃单费率，小数文本）</label>
              <ElInput v-model="observerConfig.bybit.fallback_taker_fee_rate" size="small" />
            </div>
            <div class="field-group">
              <label><TermHint term="api_key_env" />（密钥环境变量名）</label>
              <ElInput v-model="observerConfig.bybit.api_key_env" size="small" />
            </div>
            <div class="field-group">
              <label><TermHint term="api_secret_env" /></label>
              <ElInput v-model="observerConfig.bybit.api_secret_env" size="small" />
            </div>
            <div class="field-group">
              <label><TermHint term="region_eligible_confirmed" /></label>
              <ElSwitch v-model="observerConfig.bybit.region_eligible_confirmed" />
            </div>
            <div class="field-group">
              <label><TermHint term="account_eligible_confirmed" /></label>
              <ElSwitch v-model="observerConfig.bybit.account_eligible_confirmed" />
            </div>
          </div>
        </div>

        <!-- 策略 -->
        <div class="settings-card">
          <div class="card-header">
            <span class="card-title">策略（strategy）</span>
          </div>
          <div class="card-body">
            <div class="field-group">
              <label><TermHint term="min_net_profit" />（最小净利润）</label>
              <ElInput v-model="observerConfig.strategy.min_net_profit" size="small" />
            </div>
            <div class="field-group">
              <label><TermHint term="min_net_bps" />（最小净基差 bp）</label>
              <ElInput v-model="observerConfig.strategy.min_net_bps" size="small" />
            </div>
            <div class="field-group">
              <label><TermHint term="latency_loss_bps" />（延迟损失 bp）</label>
              <ElInput v-model="observerConfig.strategy.latency_loss_bps" size="small" />
            </div>
            <div class="field-group">
              <label><TermHint term="risk_buffer_bps" />（风险缓冲 bp）</label>
              <ElInput v-model="observerConfig.strategy.risk_buffer_bps" size="small" />
            </div>
            <div class="field-group">
              <label><TermHint term="rebalance_cost" />（再平衡成本）</label>
              <ElInput v-model="observerConfig.strategy.rebalance_cost" size="small" />
            </div>
            <div class="field-group">
              <label><TermHint term="other_direct_cost" />（其他直接成本）</label>
              <ElInput v-model="observerConfig.strategy.other_direct_cost" size="small" />
            </div>
          </div>
        </div>
      </div>
    </div>

    <div class="settings-actions">
      <ElButton @click="loadConfig" :loading="loading" :disabled="!canOperate">重新加载</ElButton>
      <ElButton @click="saveEnvConfig" :loading="savingEnv" :disabled="!canOperate">保存凭据</ElButton>
      <ElButton type="primary" @click="saveObserver" :loading="savingObserver" :disabled="!canOperate">
        保存观察配置
      </ElButton>
    </div>
  </section>
</template>

<style scoped>
.settings-page {
  padding: 0 0 20px;
}
.settings-header {
  margin-bottom: 18px;
}
.settings-header h2 {
  margin: 0;
  color: var(--text-1);
  font-size: var(--fs-15);
  font-weight: 700;
}
.settings-header small {
  color: var(--text-3);
  font-size: var(--fs-11);
  margin-top: 6px;
  display: block;
}
.settings-section {
  margin-bottom: 20px;
}
.settings-section h3 {
  margin: 0 0 10px;
  color: var(--text-1);
  font-size: var(--fs-13);
  font-weight: 700;
  letter-spacing: .03em;
}
.settings-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 14px;
}
.settings-card {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  overflow: hidden;
  box-shadow: var(--shadow-1);
}
.card-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 11px 15px;
  background: var(--bg-inset);
  border-bottom: 1px solid var(--border);
}
.card-title {
  font-size: var(--fs-12);
  font-weight: 700;
  color: var(--text-1);
  letter-spacing: .03em;
}
.card-body {
  padding: 14px 15px;
}
.field-group {
  margin-bottom: 13px;
}
.field-group:last-child {
  margin-bottom: 0;
}
.field-group label {
  display: block;
  margin-bottom: 5px;
  color: var(--text-2);
  font-size: var(--fs-11);
  font-weight: 600;
  letter-spacing: .03em;
  font-family: var(--font-mono);
}
.field-group small {
  display: block;
  margin-top: 5px;
  color: var(--text-3);
  font-size: var(--fs-10);
}
.settings-actions {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  padding: 14px 0;
  border-top: 1px solid var(--border);
}
.pair-add-select {
  width: 220px;
  text-align: left;
}
.pair-editor {
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 10px 12px;
  margin-bottom: 10px;
  background: var(--bg-inset);
}
.pair-editor:last-child {
  margin-bottom: 0;
}
.pair-editor-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 8px;
}
.pair-editor-head b {
  color: var(--text-1);
  font-size: var(--fs-12);
  font-family: var(--font-mono);
}
.pair-editor-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 10px;
}
.pair-editor-grid .field-group {
  margin-bottom: 0;
}
</style>
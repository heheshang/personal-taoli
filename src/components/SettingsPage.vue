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
import type { AppConfig, ObserverConfig } from '../commands'

const props = defineProps<{
  canOperate: boolean
}>()

const emit = defineEmits<{
  configSaved: [config: AppConfig]
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
      // 后端总是返回完整对象：直接覆盖默认模板
      Object.assign(observerConfig, observerData)
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
    }
  } finally {
    savingObserver.value = false
  }
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
              <label>database_url</label>
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
              <label>binance_api_key</label>
              <ElInput v-model="envConfig.binance_api_key" size="small" show-password placeholder="Binance API Key" />
            </div>
            <div class="field-group">
              <label>binance_api_secret</label>
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
              <label>bybit_api_key</label>
              <ElInput v-model="envConfig.bybit_api_key" size="small" show-password placeholder="Bybit API Key" />
            </div>
            <div class="field-group">
              <label>bybit_api_secret</label>
              <ElInput v-model="envConfig.bybit_api_secret" size="small" show-password placeholder="Bybit API Secret" />
            </div>
          </div>
        </div>
      </div>
    </div>

    <div class="settings-section">
      <h3>观察配置（ObserverConfig）</h3>
      <div class="settings-grid">
        <!-- 交易对与数量 -->
        <div class="settings-card">
          <div class="card-header">
            <span class="card-title">交易对与数量</span>
          </div>
          <div class="card-body">
            <div class="field-group">
              <label>symbol</label>
              <ElInput v-model="observerConfig.symbol" size="small" />
            </div>
            <div class="field-group">
              <label>base_asset</label>
              <ElInput v-model="observerConfig.base_asset" size="small" />
            </div>
            <div class="field-group">
              <label>quote_asset</label>
              <ElInput v-model="observerConfig.quote_asset" size="small" />
            </div>
            <div class="field-group">
              <label>quantity（数量，小数文本）</label>
              <ElInput v-model="observerConfig.quantity" size="small" />
            </div>
            <div class="field-group">
              <label>orderbook_depth（档位）</label>
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
              <label>poll_interval_ms（轮询间隔）</label>
              <ElInputNumber v-model="observerConfig.poll_interval_ms" size="small" :min="0" controls-position="right" />
            </div>
            <div class="field-group">
              <label>http_timeout_ms（HTTP 超时）</label>
              <ElInputNumber v-model="observerConfig.http_timeout_ms" size="small" :min="0" controls-position="right" />
            </div>
            <div class="field-group">
              <label>stream_start_timeout_ms（流启动超时）</label>
              <ElInputNumber v-model="observerConfig.stream_start_timeout_ms" size="small" :min="0" controls-position="right" />
            </div>
            <div class="field-group">
              <label>reconnect_delay_ms（重连延迟）</label>
              <ElInputNumber v-model="observerConfig.reconnect_delay_ms" size="small" :min="0" controls-position="right" />
            </div>
            <div class="field-group">
              <label>max_snapshot_age_ms（快照最大年龄）</label>
              <ElInputNumber v-model="observerConfig.max_snapshot_age_ms" size="small" :min="0" controls-position="right" />
            </div>
            <div class="field-group">
              <label>max_pair_skew_ms（价差最大年龄）</label>
              <ElInputNumber v-model="observerConfig.max_pair_skew_ms" size="small" :min="0" controls-position="right" />
            </div>
            <div class="field-group">
              <label>account_refresh_interval_ms（账户刷新间隔）</label>
              <ElInputNumber v-model="observerConfig.account_refresh_interval_ms" size="small" :min="0" controls-position="right" />
            </div>
            <div class="field-group">
              <label>max_fee_age_ms（费率最大年龄）</label>
              <ElInputNumber v-model="observerConfig.max_fee_age_ms" size="small" :min="0" controls-position="right" />
            </div>
            <div class="field-group">
              <label>auth_recv_window_ms（签名窗口）</label>
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
              <label>queue_capacity（队列容量）</label>
              <ElInputNumber v-model="observerConfig.archive.queue_capacity" size="small" :min="1" controls-position="right" />
            </div>
            <div class="field-group">
              <label>raw_retention_days（原始保留天数）</label>
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
              <label>fallback_taker_fee_rate（备用吃单费率，小数文本）</label>
              <ElInput v-model="observerConfig.binance.fallback_taker_fee_rate" size="small" />
            </div>
            <div class="field-group">
              <label>api_key_env（密钥环境变量名）</label>
              <ElInput v-model="observerConfig.binance.api_key_env" size="small" />
            </div>
            <div class="field-group">
              <label>api_secret_env</label>
              <ElInput v-model="observerConfig.binance.api_secret_env" size="small" />
            </div>
            <div class="field-group">
              <label>region_eligible_confirmed</label>
              <ElSwitch v-model="observerConfig.binance.region_eligible_confirmed" />
            </div>
            <div class="field-group">
              <label>account_eligible_confirmed</label>
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
              <label>fallback_taker_fee_rate（备用吃单费率，小数文本）</label>
              <ElInput v-model="observerConfig.bybit.fallback_taker_fee_rate" size="small" />
            </div>
            <div class="field-group">
              <label>api_key_env（密钥环境变量名）</label>
              <ElInput v-model="observerConfig.bybit.api_key_env" size="small" />
            </div>
            <div class="field-group">
              <label>api_secret_env</label>
              <ElInput v-model="observerConfig.bybit.api_secret_env" size="small" />
            </div>
            <div class="field-group">
              <label>region_eligible_confirmed</label>
              <ElSwitch v-model="observerConfig.bybit.region_eligible_confirmed" />
            </div>
            <div class="field-group">
              <label>account_eligible_confirmed</label>
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
              <label>min_net_profit（最小净利润）</label>
              <ElInput v-model="observerConfig.strategy.min_net_profit" size="small" />
            </div>
            <div class="field-group">
              <label>min_net_bps（最小净基差 bp）</label>
              <ElInput v-model="observerConfig.strategy.min_net_bps" size="small" />
            </div>
            <div class="field-group">
              <label>latency_loss_bps（延迟损失 bp）</label>
              <ElInput v-model="observerConfig.strategy.latency_loss_bps" size="small" />
            </div>
            <div class="field-group">
              <label>risk_buffer_bps（风险缓冲 bp）</label>
              <ElInput v-model="observerConfig.strategy.risk_buffer_bps" size="small" />
            </div>
            <div class="field-group">
              <label>rebalance_cost（再平衡成本）</label>
              <ElInput v-model="observerConfig.strategy.rebalance_cost" size="small" />
            </div>
            <div class="field-group">
              <label>other_direct_cost（其他直接成本）</label>
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
  margin-bottom: 16px;
}
.settings-header h2 {
  margin: 0;
  color: #3c5a4a;
  font-size: 16px;
  font-weight: 700;
}
.settings-header small {
  color: #9ca79f;
  font-size: 10px;
}
.settings-section {
  margin-bottom: 18px;
}
.settings-section h3 {
  margin: 0 0 8px;
  color: #3c5a4a;
  font-size: 12px;
  font-weight: 700;
  letter-spacing: .03em;
}
.settings-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 12px;
}
.settings-card {
  background: #fbfcfa;
  border: 1px solid #d7ddd6;
  border-radius: 8px;
  overflow: hidden;
}
.card-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 10px 14px;
  background: #f3f7f2;
  border-bottom: 1px solid #e2e7e2;
}
.card-title {
  font-size: 11px;
  font-weight: 700;
  color: #3c5a4a;
  letter-spacing: .03em;
}
.card-body {
  padding: 12px 14px;
}
.field-group {
  margin-bottom: 12px;
}
.field-group:last-child {
  margin-bottom: 0;
}
.field-group label {
  display: block;
  margin-bottom: 4px;
  color: #59675e;
  font-size: 10px;
  font-weight: 600;
  letter-spacing: .03em;
}
.field-group small {
  display: block;
  margin-top: 4px;
  color: #9ca79f;
  font-size: 8px;
}
.settings-actions {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  padding: 12px 0;
  border-top: 1px solid #e2e7e2;
}
</style>
<script setup lang="ts">
import { ref, onMounted, reactive } from 'vue'
import { ElButton, ElMessage, ElInput, ElInputNumber, ElSwitch } from 'element-plus'
import { invoke } from '@tauri-apps/api/core'

const props = defineProps<{
  canOperate: boolean
}>()

const emit = defineEmits<{
  configSaved: []
}>()

interface AppConfig {
  database_url: string
  binance_api_key: string
  binance_api_secret: string
  bybit_api_key: string
  bybit_api_secret: string
}

interface TomlConfig {
  symbol: string
  base_asset: string
  quote_asset: string
  quantity: string
  poll_interval_ms: number
  http_timeout_ms: number
  stream_start_timeout_ms: number
  reconnect_delay_ms: number
  max_snapshot_age_ms: number
  max_pair_skew_ms: number
  orderbook_depth: number
  account_refresh_interval_ms: number
  max_fee_age_ms: number
  auth_recv_window_ms: number
  archive: {
    path: string
    queue_capacity: number
    raw_retention_days: number
  }
  binance: {
    base_url: string
    websocket_url: string
    fallback_taker_fee_rate: string
    region_eligible_confirmed: boolean
    account_eligible_confirmed: boolean
  }
  bybit: {
    base_url: string
    websocket_url: string
    fallback_taker_fee_rate: string
    region_eligible_confirmed: boolean
    account_eligible_confirmed: boolean
  }
  strategy: {
    min_net_profit: string
    min_net_bps: string
    latency_loss_bps: string
    risk_buffer_bps: string
    rebalance_cost: string
    other_direct_cost: string
  }
}

const loading = ref(false)
const saving = ref(false)

const envConfig = reactive<AppConfig>({
  database_url: '',
  binance_api_key: '',
  binance_api_secret: '',
  bybit_api_key: '',
  bybit_api_secret: '',
})

async function loadConfig() {
  loading.value = true
  try {
    const result = await invoke<{ success: boolean; data: AppConfig }>('load_app_config')
    if (result.success && result.data) {
      Object.assign(envConfig, result.data)
    }
  } catch (error) {
    ElMessage.error(`加载配置失败: ${String(error)}`)
  } finally {
    loading.value = false
  }
}

async function saveConfig() {
  saving.value = true
  try {
    const result = await invoke<{ success: boolean }>('save_app_config', { config: { ...envConfig } })
    if (result.success) {
      ElMessage.success('配置已保存')
      emit('configSaved')
    }
  } catch (error) {
    ElMessage.error(`保存配置失败: ${String(error)}`)
  } finally {
    saving.value = false
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
      <small>配置数据库和交易所 API 凭证</small>
    </div>

    <div class="settings-grid">
      <!-- 数据库 -->
      <div class="settings-card">
        <div class="card-header">
          <span class="card-title">数据库</span>
        </div>
        <div class="card-body">
          <div class="field-group">
            <label>TAOLI_DATABASE_URL</label>
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
            <label>TAOLI_BINANCE_API_KEY</label>
            <ElInput v-model="envConfig.binance_api_key" size="small" show-password placeholder="Binance API Key" />
          </div>
          <div class="field-group">
            <label>TAOLI_BINANCE_API_SECRET</label>
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
            <label>TAOLI_BYBIT_API_KEY</label>
            <ElInput v-model="envConfig.bybit_api_key" size="small" show-password placeholder="Bybit API Key" />
          </div>
          <div class="field-group">
            <label>TAOLI_BYBIT_API_SECRET</label>
            <ElInput v-model="envConfig.bybit_api_secret" size="small" show-password placeholder="Bybit API Secret" />
          </div>
        </div>
      </div>
    </div>

    <div class="settings-actions">
      <ElButton @click="loadConfig" :loading="loading">重新加载</ElButton>
      <ElButton type="primary" @click="saveConfig" :loading="saving">保存配置</ElButton>
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
.settings-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 12px;
  margin-bottom: 16px;
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

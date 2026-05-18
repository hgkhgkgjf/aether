<template>
  <PageContainer>
    <PageHeader
      title="重要通知"
      description="配置后台任务使用的邮件和 Server 酱通知通道"
    />

    <div class="mt-6 space-y-6">
      <CardSection
        title="模块开关"
        description="启用后，额度提醒等后台任务可以发送重要通知"
      >
        <template #actions>
          <Button
            size="sm"
            :disabled="saving"
            @click="saveConfig"
          >
            {{ saving ? '保存中...' : '保存' }}
          </Button>
        </template>

        <div class="flex items-center justify-between gap-4">
          <div>
            <Label class="text-sm font-medium">
              启用重要通知
            </Label>
            <p class="mt-1 text-xs text-muted-foreground">
              {{ anyChannelConfigurable
                ? '至少配置一个可用通道后再启用'
                : '请先完成邮件或 Server 酱通道配置后再启用'
              }}
            </p>
          </div>
          <Switch
            v-model="config.enabled"
            :disabled="!anyChannelConfigurable"
          />
        </div>
      </CardSection>

      <CardSection
        title="邮件通知"
        description="使用系统 SMTP 配置向固定收件人发送提醒"
      >
        <div class="space-y-4">
          <div class="flex items-center justify-between gap-4">
            <div>
              <Label class="text-sm font-medium">
                启用邮件通道
              </Label>
              <p
                v-if="emailChannelConfigurable"
                class="mt-1 text-xs text-muted-foreground"
              >
                SMTP 服务在邮件配置中维护
              </p>
              <p
                v-else
                class="mt-1 text-xs text-destructive"
              >
                <template v-if="!smtpConfigured">
                  请先在
                  <RouterLink
                    to="/admin/email"
                    class="hover:underline"
                  >
                    邮件配置
                  </RouterLink>
                  中配置 SMTP，
                </template>
                <template v-else>
                  请先
                </template>
                填写至少一个收件人后再启用
              </p>
            </div>
            <Switch
              v-model="config.email_enabled"
              :disabled="!emailChannelConfigurable"
            />
          </div>

          <div>
            <Label
              for="important-notification-recipients"
              class="block text-sm font-medium"
            >
              收件人
            </Label>
            <Textarea
              id="important-notification-recipients"
              v-model="config.email_recipients"
              rows="4"
              placeholder="ops@example.com&#10;admin@example.com"
              class="mt-1"
            />
            <p class="mt-1 text-xs text-muted-foreground">
              支持换行、逗号或分号分隔
            </p>
          </div>
        </div>
      </CardSection>

      <CardSection
        title="Server 酱"
        description="通过 Server 酱 Turbo SendKey 推送微信提醒"
      >
        <div class="space-y-4">
          <div class="flex items-center justify-between gap-4">
            <div>
              <Label class="text-sm font-medium">
                启用 Server 酱通道
              </Label>
              <p
                v-if="serverChanKeyIsSet"
                class="mt-1 text-xs text-muted-foreground"
              >
                请求地址使用 Server 酱 Turbo 官方接口
              </p>
              <p
                v-else
                class="mt-1 text-xs text-destructive"
              >
                请先前往
                <RouterLink
                  to="/admin/server-chan"
                  class="hover:underline"
                >
                  Server 酱
                </RouterLink>
                配置 SendKey 后再启用
              </p>
            </div>
            <Switch
              v-model="config.server_chan_enabled"
              :disabled="!serverChanKeyIsSet"
            />
          </div>

          <p
            v-if="serverChanKeyIsSet"
            class="text-xs text-muted-foreground"
          >
            前往
            <RouterLink
              to="/admin/server-chan"
              class="text-primary hover:underline"
            >
              Server 酱
            </RouterLink>
            配置 SendKey 与通知模板。
          </p>
        </div>
      </CardSection>

      <CardSection
        title="测试通知"
        description="按当前已保存配置发送一条重要通知测试"
      >
        <div class="flex flex-wrap gap-2">
          <Button
            variant="outline"
            :disabled="testingAll || !anyChannelConfigurable"
            @click="testChannel('all')"
          >
            {{ testingAll ? '发送中...' : '测试全部通道' }}
          </Button>
          <Button
            variant="outline"
            :disabled="testingEmail || !emailChannelConfigurable"
            @click="testChannel('email')"
          >
            {{ testingEmail ? '发送中...' : '测试邮件' }}
          </Button>
        </div>

        <div
          v-if="lastTestResult.length > 0"
          class="mt-4 space-y-2"
        >
          <div
            v-for="item in lastTestResult"
            :key="item.channel"
            class="flex items-center justify-between rounded-md border border-border px-3 py-2 text-sm"
          >
            <span>{{ formatChannel(item.channel) }}</span>
            <span :class="item.success ? 'text-green-600 dark:text-green-400' : 'text-destructive'">
              {{ item.message }}
            </span>
          </div>
        </div>
      </CardSection>
    </div>
  </PageContainer>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { RouterLink } from 'vue-router'
import Button from '@/components/ui/button.vue'
import Label from '@/components/ui/label.vue'
import Switch from '@/components/ui/switch.vue'
import Textarea from '@/components/ui/textarea.vue'
import { PageHeader, PageContainer, CardSection } from '@/components/layout'
import { adminApi } from '@/api/admin'
import { modulesApi } from '@/api/modules'
import { useToast } from '@/composables/useToast'
import { parseApiError } from '@/utils/errorParser'
import { log } from '@/utils/logger'

const CONFIG_KEYS = {
  enabled: 'module.important_notification.enabled',
  email_enabled: 'module.important_notification.email_enabled',
  email_recipients: 'module.important_notification.email_recipients',
  server_chan_enabled: 'module.important_notification.server_chan_enabled',
  server_chan_send_key: 'module.important_notification.server_chan_send_key',
} as const

interface ImportantNotificationConfig {
  enabled: boolean
  email_enabled: boolean
  email_recipients: string
  server_chan_enabled: boolean
}

const { success, error } = useToast()

const saving = ref(false)
const testingAll = ref(false)
const testingEmail = ref(false)
const lastTestResult = ref<Array<{ channel: string; success: boolean; message: string }>>([])

const smtpConfigured = ref(false)
const serverChanKeyIsSet = ref(false)

const config = ref<ImportantNotificationConfig>({
  enabled: false,
  email_enabled: false,
  email_recipients: '',
  server_chan_enabled: false,
})

const emailChannelConfigurable = computed(() => {
  return smtpConfigured.value && config.value.email_recipients.trim() !== ''
})

const anyChannelConfigurable = computed(() => {
  return emailChannelConfigurable.value || serverChanKeyIsSet.value
})

onMounted(() => {
  loadConfig()
})

async function loadConfig() {
  try {
    const [
      moduleStatus,
      emailEnabled,
      recipients,
      serverChanEnabled,
      serverChanKey,
      smtpHost,
      smtpFromEmail,
    ] = await Promise.all([
      modulesApi.getStatus('important_notification'),
      adminApi.getSystemConfig(CONFIG_KEYS.email_enabled),
      adminApi.getSystemConfig(CONFIG_KEYS.email_recipients),
      adminApi.getSystemConfig(CONFIG_KEYS.server_chan_enabled),
      adminApi.getSystemConfig(CONFIG_KEYS.server_chan_send_key),
      adminApi.getSystemConfig('smtp_host'),
      adminApi.getSystemConfig('smtp_from_email'),
    ])

    config.value.enabled = moduleStatus.enabled === true
    config.value.email_enabled = emailEnabled.value === true
    config.value.email_recipients = normalizeRecipients(recipients.value)
    config.value.server_chan_enabled = serverChanEnabled.value === true
    serverChanKeyIsSet.value = serverChanKey.is_set === true
    smtpConfigured.value = isNonEmptyString(smtpHost.value) && isNonEmptyString(smtpFromEmail.value)
  } catch (err) {
    error(parseApiError(err, '加载重要通知配置失败'))
    log.error('加载重要通知配置失败:', err)
  }
}

async function saveConfig() {
  saving.value = true
  try {
    if (!config.value.enabled) {
      await adminApi.updateSystemConfig(CONFIG_KEYS.enabled, false, '重要通知模块总开关')
    }

    await Promise.all([
      adminApi.updateSystemConfig(CONFIG_KEYS.email_enabled, config.value.email_enabled, '重要通知邮件通道开关'),
      adminApi.updateSystemConfig(CONFIG_KEYS.email_recipients, config.value.email_recipients, '重要通知邮件收件人'),
      adminApi.updateSystemConfig(CONFIG_KEYS.server_chan_enabled, config.value.server_chan_enabled, '重要通知 Server 酱通道开关'),
    ])
    if (config.value.enabled) {
      await adminApi.updateSystemConfig(CONFIG_KEYS.enabled, true, '重要通知模块总开关')
    }
    success('重要通知配置已保存')
  } catch (err) {
    error(parseApiError(err, '保存重要通知配置失败'))
    log.error('保存重要通知配置失败:', err)
  } finally {
    saving.value = false
  }
}

async function testChannel(channel: 'all' | 'email') {
  setTesting(channel, true)
  try {
    const result = await adminApi.testImportantNotification(channel)
    lastTestResult.value = result.channels || []
    if (result.success) {
      success(result.message || '测试通知已发送')
    } else {
      error(result.message || '测试通知发送失败')
    }
  } catch (err) {
    error(parseApiError(err, '测试通知发送失败'))
    log.error('测试重要通知失败:', err)
  } finally {
    setTesting(channel, false)
  }
}

function setTesting(channel: 'all' | 'email', value: boolean) {
  if (channel === 'all') testingAll.value = value
  if (channel === 'email') testingEmail.value = value
}

function isNonEmptyString(value: unknown): boolean {
  return typeof value === 'string' && value.trim() !== ''
}

function normalizeRecipients(value: unknown): string {
  if (Array.isArray(value)) {
    return value
      .map(item => String(item).trim())
      .filter(Boolean)
      .join('\n')
  }
  return typeof value === 'string' ? value : ''
}

function formatChannel(channel: string): string {
  if (channel === 'email') return '邮件'
  if (channel === 'server_chan') return 'Server 酱'
  if (channel === 'module') return '模块'
  return channel
}
</script>

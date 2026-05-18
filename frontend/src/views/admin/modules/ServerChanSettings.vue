<template>
  <PageContainer>
    <PageHeader
      title="Server 酱"
      description="配置 Server 酱 Turbo SendKey 与微信通知模板"
    />

    <div class="mt-6 space-y-6">
      <CardSection
        title="SendKey"
        description="使用 Server 酱 Turbo 官方 SendKey 推送微信通知"
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

        <div>
          <Label
            for="server-chan-send-key"
            class="block text-sm font-medium"
          >
            SendKey
          </Label>
          <Input
            id="server-chan-send-key"
            v-model="sendKeyInput"
            masked
            :placeholder="sendKeyIsSet ? '已设置（留空保持不变）' : 'SCTxxxxxxxxxxxxxxxxxxxxxxxx'"
            class="mt-1"
          />
          <p class="mt-1 text-xs text-muted-foreground">
            可在 <span class="font-mono">sct.ftqq.com</span> 控制台获取
          </p>
        </div>
      </CardSection>

      <CardSection
        title="通知模板"
        description="可选 Markdown 模板，支持 {title} 和 {body} 变量；留空则使用默认正文"
      >
        <div>
          <Label
            for="server-chan-template"
            class="block text-sm font-medium"
          >
            模板内容
          </Label>
          <textarea
            id="server-chan-template"
            v-model="templateInput"
            rows="10"
            class="mt-1 w-full font-mono text-sm bg-muted/30 border border-border rounded-md p-3 focus:outline-none focus:ring-2 focus:ring-primary focus:border-transparent resize-y"
            placeholder="**{title}**&#10;&#10;{body}"
            spellcheck="false"
          />
          <p class="mt-2 text-xs text-muted-foreground">
            示例：<span class="font-mono">**{title}**\n\n{body}\n\n来自 Aether</span>
          </p>
        </div>
      </CardSection>

      <CardSection
        title="测试 Server 酱"
        description="按当前已保存配置向微信发送一条测试通知"
      >
        <div class="flex flex-wrap gap-2">
          <Button
            variant="outline"
            :disabled="testing"
            @click="handleTest"
          >
            {{ testing ? '发送中...' : '测试 Server 酱' }}
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
import { onMounted, ref } from 'vue'
import Button from '@/components/ui/button.vue'
import Input from '@/components/ui/input.vue'
import Label from '@/components/ui/label.vue'
import { PageHeader, PageContainer, CardSection } from '@/components/layout'
import { adminApi } from '@/api/admin'
import { useToast } from '@/composables/useToast'
import { parseApiError } from '@/utils/errorParser'
import { log } from '@/utils/logger'

const CONFIG_KEYS = {
  server_chan_send_key: 'module.important_notification.server_chan_send_key',
  server_chan_template: 'module.important_notification.server_chan_template',
} as const

const { success, error } = useToast()

const saving = ref(false)
const testing = ref(false)
const sendKeyIsSet = ref(false)
const sendKeyInput = ref('')
const templateInput = ref('')
const lastTestResult = ref<Array<{ channel: string; success: boolean; message: string }>>([])

onMounted(() => {
  loadConfig()
})

async function loadConfig() {
  try {
    const [sendKey, template] = await Promise.all([
      adminApi.getSystemConfig(CONFIG_KEYS.server_chan_send_key),
      adminApi.getSystemConfig(CONFIG_KEYS.server_chan_template),
    ])

    sendKeyIsSet.value = sendKey.is_set === true
    sendKeyInput.value = ''
    templateInput.value = typeof template.value === 'string' ? template.value : ''
  } catch (err) {
    error(parseApiError(err, '加载 Server 酱配置失败'))
    log.error('加载 Server 酱配置失败:', err)
  }
}

async function saveConfig() {
  saving.value = true
  try {
    const updates: Array<Promise<unknown>> = [
      adminApi.updateSystemConfig(
        CONFIG_KEYS.server_chan_template,
        templateInput.value,
        '重要通知 Server 酱 通知模板',
      ),
    ]
    const trimmedKey = sendKeyInput.value.trim()
    if (trimmedKey) {
      updates.push(
        adminApi.updateSystemConfig(
          CONFIG_KEYS.server_chan_send_key,
          trimmedKey,
          '重要通知 Server 酱 SendKey',
        ),
      )
    }
    await Promise.all(updates)
    if (trimmedKey) {
      sendKeyIsSet.value = true
      sendKeyInput.value = ''
    }
    success('Server 酱配置已保存')
  } catch (err) {
    error(parseApiError(err, '保存 Server 酱配置失败'))
    log.error('保存 Server 酱配置失败:', err)
  } finally {
    saving.value = false
  }
}

async function handleTest() {
  testing.value = true
  try {
    const result = await adminApi.testImportantNotification('server_chan')
    lastTestResult.value = result.channels || []
    if (result.success) {
      success(result.message || '测试通知已发送')
    } else {
      error(result.message || '测试通知发送失败')
    }
  } catch (err) {
    error(parseApiError(err, '测试通知发送失败'))
    log.error('测试 Server 酱失败:', err)
  } finally {
    testing.value = false
  }
}

function formatChannel(channel: string): string {
  if (channel === 'server_chan') return 'Server 酱'
  if (channel === 'email') return '邮件'
  if (channel === 'module') return '模块'
  return channel
}
</script>

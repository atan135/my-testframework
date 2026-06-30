<template>
  <el-dialog
    class="result-detail-dialog"
    :model-value="modelValue"
    title="返回详情"
    width="min(920px, 92vw)"
    @update:model-value="emit('update:modelValue', $event)"
  >
    <div class="result-detail-meta">
      <div v-for="item in metaItems" :key="item.label" class="result-detail-meta-item">
        <span class="result-detail-meta-label">{{ item.label }}</span>
        <div class="result-detail-meta-row">
          <strong class="result-detail-meta-value" :title="item.value">{{ item.value }}</strong>
          <el-button
            v-if="item.copyable"
            class="result-detail-meta-copy"
            :disabled="!item.copyValue"
            :icon="CopyDocument"
            link
            type="primary"
            @click="copyMetaValue(item)"
          />
        </div>
      </div>
    </div>

    <div class="result-detail-section">
      <span class="result-detail-section-title">请求参数</span>
      <pre class="result-detail-output result-detail-arguments">{{ argumentText }}</pre>
    </div>

    <div class="result-detail-section">
      <span class="result-detail-section-title">返回内容</span>
      <pre class="result-detail-output">{{ detailText }}</pre>
    </div>

    <template #footer>
      <div class="result-detail-footer">
        <el-button :icon="CopyDocument" @click="copyDetails">复制内容</el-button>
        <el-button type="primary" @click="emit('update:modelValue', false)">关闭</el-button>
      </div>
    </template>
  </el-dialog>
</template>

<script setup>
import { computed } from 'vue';
import { ElMessage } from 'element-plus/es/components/message/index.mjs';
import { CopyDocument } from '@element-plus/icons-vue';

import {
  formatArgumentsDetails,
  formatOutputDetails,
  formatTime,
  recordClientIp,
  recordClientIpList,
  recordClientName,
} from '../utils/qaFormatters';

const props = defineProps({
  modelValue: {
    type: Boolean,
    required: true,
  },
  record: {
    type: Object,
    default: null,
  },
});

const emit = defineEmits(['update:modelValue']);

const detailText = computed(() => formatOutputDetails(props.record));
const argumentText = computed(() => formatArgumentsDetails(props.record));
const metaItems = computed(() => {
  const record = props.record || {};
  const clientName = recordClientName(record);
  const clientIp = recordClientIp(record);
  const clientIps = recordClientIpList(record);
  return [
    { label: '状态', value: record.status || '-' },
    { label: '方法', value: record.methodName || '-' },
    { label: '客户端', value: clientName || record.clientId || '-' },
    { label: 'Client ID', value: record.clientId || '-' },
    { label: 'IP', value: clientIp || '-' },
    {
      label: '全部 IP',
      value: clientIps.length > 0 ? clientIps.join(', ') : '-',
      copyable: true,
      copyValue: clientIps.join(', '),
    },
    { label: '请求 ID', value: record.requestId || '-' },
    { label: '耗时', value: record.durationMs ? `${record.durationMs} ms` : '-' },
    { label: '时间', value: formatTime(record.finishedAt || record.startedAt) },
  ];
});

async function copyDetails() {
  try {
    await writeClipboard(
      [
        '请求参数:',
        argumentText.value,
        '',
        '返回内容:',
        detailText.value,
      ].join('\n'),
    );
    ElMessage.success('已复制');
  } catch (error) {
    ElMessage.error(error.message || '复制失败');
  }
}

async function copyMetaValue(item) {
  if (!item.copyValue) {
    return;
  }

  try {
    await writeClipboard(item.copyValue);
    ElMessage.success(`已复制${item.label}`);
  } catch (error) {
    ElMessage.error(error.message || '复制失败');
  }
}

async function writeClipboard(value) {
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(value);
    return;
  }

  const textarea = document.createElement('textarea');
  textarea.value = value;
  textarea.setAttribute('readonly', '');
  textarea.style.position = 'fixed';
  textarea.style.opacity = '0';
  document.body.appendChild(textarea);
  textarea.select();
  const copied = document.execCommand('copy');
  document.body.removeChild(textarea);
  if (!copied) {
    throw new Error('浏览器拒绝复制');
  }
}
</script>

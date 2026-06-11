<template>
  <div class="page-view client-console-view">
    <header class="topbar">
      <div>
        <p class="eyebrow">当前目标</p>
        <h2>{{ selectedClient ? selectedClient.name : '等待 Unity 连接' }}</h2>
      </div>
      <div class="topbar-actions">
        <el-button :icon="Refresh" @click="refreshAll">刷新</el-button>
      </div>
    </header>

    <section v-if="selectedClient" class="workspace-grid">
      <section class="section client-picker">
        <div class="section-heading">
          <div>
            <h3>Unity 实例</h3>
          </div>
          <el-tag effect="plain">{{ clients.length }}</el-tag>
        </div>

        <el-scrollbar class="client-list">
          <button
            v-for="client in clients"
            :key="client.clientId"
            class="client-item"
            :class="{ active: client.clientId === selectedClientId }"
            @click="selectedClientId = client.clientId"
          >
            <span class="client-name">{{ client.name }}</span>
            <span class="client-meta">{{ client.platform }} · {{ clientIpText(client) }} · {{ client.methods?.length || 0 }} methods</span>
          </button>
        </el-scrollbar>
      </section>

      <div class="client-workspace">
        <section class="section selected-client-panel">
          <div class="section-heading">
            <div>
              <h3>选中实例信息</h3>
            </div>
            <el-tag :type="selectedClient.available === false ? 'danger' : 'success'" effect="plain">{{ clientStatusText }}</el-tag>
          </div>
          <div class="metric-row">
            <div class="metric">
              <span>客户端 ID</span>
              <strong>{{ selectedClient.clientId }}</strong>
            </div>
            <div class="metric">
              <span>Unity</span>
              <strong>{{ selectedClient.unityVersion || '-' }}</strong>
            </div>
            <div class="metric">
              <span>IP</span>
              <strong>{{ clientIpText(selectedClient) }}</strong>
            </div>
            <div class="metric">
              <span>最后心跳</span>
              <strong>{{ formatTime(selectedClient.lastSeenAt) }}</strong>
            </div>
            <div class="metric">
              <span>状态</span>
              <strong>{{ clientStatusText }}</strong>
            </div>
            <div class="metric">
              <span>控制锁</span>
              <strong>{{ clientLockText }}</strong>
            </div>
          </div>
        </section>

        <section class="section methods-panel">
          <div class="section-heading">
            <div>
              <h3>可执行方法</h3>
            </div>
          </div>

          <el-table :data="selectedClient.methods || []" height="420" empty-text="没有发现 [QaTest] 方法">
            <el-table-column label="方法名" min-width="220" show-overflow-tooltip>
              <template #default="{ row }">
                <span class="method-name-text">{{ methodRealName(row) }}</span>
              </template>
            </el-table-column>
            <el-table-column label="显示名" min-width="200" show-overflow-tooltip>
              <template #default="{ row }">
                <span class="method-display-name-text">{{ methodDisplayName(row) }}</span>
              </template>
            </el-table-column>
            <el-table-column label="描述" min-width="320" show-overflow-tooltip>
              <template #default="{ row }">
                <span class="method-description-text">{{ methodDescription(row) }}</span>
              </template>
            </el-table-column>
            <el-table-column label="操作" width="230" fixed="right">
              <template #default="{ row }">
                <div class="method-actions">
                  <el-button size="small" :icon="View" @click="openMethodDetails(row)">详情</el-button>
                  <el-button size="small" type="primary" :icon="VideoPlay" :disabled="!canControlSelectedClient" @click="prepareExecute(row)">执行</el-button>
                  <el-button size="small" :icon="DocumentCopy" @click="prepareExport(row)">导出</el-button>
                </div>
              </template>
            </el-table-column>
          </el-table>
        </section>
      </div>
    </section>

    <el-empty v-else class="empty-state" description="启动 Unity 后会自动注册到这里" />

    <el-dialog v-model="argumentDialogVisible" :title="argumentDialogTitle" width="520px">
      <el-form label-position="top">
        <el-form-item v-for="parameter in pendingParameters" :key="parameter.name" :label="parameterLabel(parameter)">
          <el-input v-model="argumentValues[parameter.name]" :placeholder="parameter.defaultValue || ''" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="argumentDialogVisible = false">取消</el-button>
        <el-button type="primary" :icon="argumentDialogPrimaryIcon" @click="submitPendingAction">{{ argumentDialogPrimaryText }}</el-button>
      </template>
    </el-dialog>

    <el-dialog v-model="exportDialogVisible" title="HTTP 请求指令" width="760px">
      <div class="export-request-grid">
        <label class="export-request-block">
          <span>HTTP 请求</span>
          <el-input :model-value="exportedRequest.rawHttp" type="textarea" :autosize="{ minRows: 8, maxRows: 14 }" readonly />
        </label>
        <label class="export-request-block">
          <span>PowerShell</span>
          <el-input :model-value="exportedRequest.powerShell" type="textarea" :autosize="{ minRows: 8, maxRows: 14 }" readonly />
        </label>
      </div>
      <template #footer>
        <el-button :icon="CopyDocument" @click="copyExportText(exportedRequest.rawHttp)">复制 HTTP</el-button>
        <el-button type="primary" :icon="CopyDocument" @click="copyExportText(exportedRequest.powerShell)">复制 PowerShell</el-button>
      </template>
    </el-dialog>

    <el-dialog v-model="methodDetailDialogVisible" title="方法详情" width="820px">
      <div v-if="selectedMethod" class="method-detail">
        <div class="method-detail-grid">
          <div class="method-detail-item">
            <span>方法名</span>
            <strong>{{ methodRealName(selectedMethod) }}</strong>
          </div>
          <div class="method-detail-item">
            <span>显示名</span>
            <strong>{{ methodDisplayName(selectedMethod) }}</strong>
          </div>
          <div class="method-detail-item">
            <span>返回类型</span>
            <strong>{{ selectedMethod.returnType || '-' }}</strong>
          </div>
          <div class="method-detail-item method-detail-wide">
            <span>类型</span>
            <strong>{{ selectedMethod.declaringType || '-' }}</strong>
          </div>
          <div class="method-detail-item method-detail-wide">
            <span>描述</span>
            <strong>{{ methodDescription(selectedMethod) }}</strong>
          </div>
          <div class="method-detail-item method-detail-wide">
            <span>方法 ID</span>
            <strong>{{ selectedMethod.id || '-' }}</strong>
          </div>
        </div>

        <div class="method-detail-heading">
          <span>参数</span>
          <el-tag effect="plain">{{ selectedMethodParameters.length }}</el-tag>
        </div>
        <table class="method-detail-parameter-table">
          <thead>
            <tr>
              <th>参数名</th>
              <th>类型</th>
              <th>必填</th>
              <th>默认值</th>
              <th>参数描述</th>
            </tr>
          </thead>
          <tbody v-if="selectedMethodParameters.length > 0">
            <tr v-for="parameter in selectedMethodParameters" :key="parameter.name">
              <td class="method-detail-code">{{ parameter.name || '-' }}</td>
              <td>{{ parameter.type || '-' }}</td>
              <td>{{ parameterRequirement(parameter) }}</td>
              <td>{{ parameter.defaultValue || '-' }}</td>
              <td>{{ parameter.description || '-' }}</td>
            </tr>
          </tbody>
          <tbody v-else>
            <tr>
              <td colspan="5" class="method-detail-empty">无参数</td>
            </tr>
          </tbody>
        </table>
      </div>
      <template #footer>
        <el-button type="primary" @click="methodDetailDialogVisible = false">关闭</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup>
import { computed, ref } from 'vue';
import { ElMessage } from 'element-plus/es/components/message/index.mjs';
import { CopyDocument, DocumentCopy, Refresh, VideoPlay, View } from '@element-plus/icons-vue';

import { useQaStore } from '../composables/useQaStore';
import { formatTime, parameterLabel } from '../utils/qaFormatters';

const { clients, controller, executeMethod: sendExecute, refreshAll, selectedClient, selectedClientId } = useQaStore();

const argumentDialogVisible = ref(false);
const exportDialogVisible = ref(false);
const methodDetailDialogVisible = ref(false);
const pendingMethod = ref(null);
const pendingClient = ref(null);
const selectedMethod = ref(null);
const pendingAction = ref('execute');
const argumentValues = ref({});
const exportedRequest = ref({
  rawHttp: '',
  powerShell: '',
});
const pendingParameters = computed(() => pendingMethod.value?.parameters || []);
const selectedMethodParameters = computed(() => selectedMethod.value?.parameters || []);
const argumentDialogTitle = computed(() => (pendingAction.value === 'export' ? '导出参数' : '执行参数'));
const argumentDialogPrimaryText = computed(() => (pendingAction.value === 'export' ? '生成指令' : '执行'));
const argumentDialogPrimaryIcon = computed(() => (pendingAction.value === 'export' ? DocumentCopy : VideoPlay));
const isLockedByOther = computed(() => {
  const lock = selectedClient.value?.lock;
  return Boolean(lock && lock.ownerId !== controller.value.ownerId);
});
const canControlSelectedClient = computed(() =>
  selectedClient.value &&
  selectedClient.value.available !== false &&
  !selectedClient.value.running &&
  !selectedClient.value.clientBusy &&
  !isLockedByOther.value
);
const clientStatusText = computed(() => {
  if (!selectedClient.value) return '-';
  if (selectedClient.value.available === false) return '不可用';
  if (selectedClient.value.running) return '服务端执行中';
  if (selectedClient.value.clientBusy) return 'Unity 本地执行中';
  return '可用';
});
const clientLockText = computed(() => {
  const lock = selectedClient.value?.lock;
  if (!lock) return '未占用';
  return lock.ownerId === controller.value.ownerId ? '当前控制端' : '其他控制端';
});

function prepareExecute(method) {
  prepareMethodAction('execute', method);
}

function prepareExport(method) {
  prepareMethodAction('export', method);
}

function openMethodDetails(method) {
  selectedMethod.value = method;
  methodDetailDialogVisible.value = true;
}

function prepareMethodAction(action, method) {
  pendingClient.value = selectedClient.value;
  pendingMethod.value = method;
  pendingAction.value = action;
  argumentValues.value = {};

  for (const parameter of method.parameters || []) {
    argumentValues.value[parameter.name] = parameter.defaultValue || '';
  }

  if ((method.parameters || []).length === 0) {
    submitMethodAction(method, []);
    return;
  }

  argumentDialogVisible.value = true;
}

function submitPendingAction() {
  const args = pendingParameters.value.map((parameter) => argumentValues.value[parameter.name] ?? '');
  submitMethodAction(pendingMethod.value, args, pendingClient.value);
  argumentDialogVisible.value = false;
}

function submitMethodAction(method, args, client = pendingClient.value) {
  if (pendingAction.value === 'export') {
    showHttpRequest(client, method, args);
    return;
  }

  executeMethod(client, method, args);
}

async function executeMethod(client, method, args) {
  try {
    await sendExecute(client?.clientId, method, args);
  } catch (error) {
    ElMessage.error(error.message || '执行失败');
  }
}

function showHttpRequest(client, method, args) {
  if (!client || !method) {
    ElMessage.error('缺少 Unity 实例或方法');
    return;
  }

  exportedRequest.value = buildHttpRequestExport(client, method, args);
  exportDialogVisible.value = true;
}

function buildHttpRequestExport(client, method, args) {
  const url = `${window.location.origin}/api/unity-clients/${encodeURIComponent(client.clientId)}/execute`;
  const body = {
    methodId: method.id || method.name,
    methodName: method.name,
    arguments: args,
  };
  const bodyJson = JSON.stringify(body, null, 2);
  const parsedUrl = new URL(url);
  const rawHttp = [
    `POST ${parsedUrl.pathname}${parsedUrl.search} HTTP/1.1`,
    `Host: ${parsedUrl.host}`,
    'Content-Type: application/json',
    '',
    bodyJson,
  ].join('\n');
  const powerShell = [
    "$body = @'",
    bodyJson,
    "'@",
    `Invoke-RestMethod -Method Post -Uri "${escapePowerShellDoubleQuoted(url)}" -ContentType "application/json" -Body $body`,
  ].join('\n');

  return {
    rawHttp,
    powerShell,
  };
}

async function copyExportText(value) {
  try {
    await copyText(value);
    ElMessage.success('已复制');
  } catch (error) {
    ElMessage.error(error.message || '复制失败');
  }
}

async function copyText(value) {
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

function escapePowerShellDoubleQuoted(value) {
  return String(value)
    .replace(/`/g, '``')
    .replace(/"/g, '`"')
    .replace(/\$/g, '`$');
}

function clientIpText(client) {
  return client?.ipAddress || client?.remoteAddress || '-';
}

function methodRealName(method) {
  const directName = method?.rawName || method?.methodName || method?.csharpName;
  if (directName) {
    return directName;
  }

  const id = method?.id || '';
  const parenIndex = id.indexOf('(');
  const prefix = parenIndex >= 0 ? id.slice(0, parenIndex) : id;
  const dotIndex = prefix.lastIndexOf('.');
  const parsedName = dotIndex >= 0 ? prefix.slice(dotIndex + 1) : prefix;
  return parsedName || method?.name || '-';
}

function methodDisplayName(method) {
  return method?.name || '-';
}

function methodDescription(method) {
  const description = method?.description || '';
  return description.trim() || '-';
}

function parameterRequirement(parameter) {
  return parameter?.isOptional ? '可选' : '必填';
}
</script>

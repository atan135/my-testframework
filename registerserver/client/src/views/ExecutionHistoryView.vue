<template>
  <div class="page-view">
    <header class="topbar">
      <div>
        <p class="eyebrow">History</p>
        <h2>执行记录</h2>
      </div>
      <div class="topbar-actions">
        <el-button :icon="Refresh" @click="refreshAll">刷新</el-button>
      </div>
    </header>

    <div class="metric-row">
      <div class="metric">
        <span>总记录</span>
        <strong>{{ history.length }}</strong>
      </div>
      <div class="metric">
        <span>成功</span>
        <strong>{{ successCount }}</strong>
      </div>
      <div class="metric">
        <span>失败</span>
        <strong>{{ failedCount }}</strong>
      </div>
    </div>

    <section class="section">
      <div class="section-heading">
        <div>
          <p class="eyebrow">Results</p>
          <h3>最近结果</h3>
        </div>
      </div>

      <el-table :data="history" height="560" empty-text="暂无执行记录">
        <el-table-column prop="status" label="状态" width="110">
          <template #default="{ row }">
            <el-tag :type="historyStatusType(row.status)" effect="plain">{{ row.status }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column prop="methodName" label="方法" min-width="180" show-overflow-tooltip />
        <el-table-column prop="clientId" label="客户端" min-width="180" show-overflow-tooltip />
        <el-table-column label="输出" min-width="360">
          <template #default="{ row }">
            <div class="result-cell">
              <span class="result-preview">{{ formatOutputPreview(row) }}</span>
              <el-button type="primary" link :icon="View" @click="openResultDetails(row)">查看</el-button>
            </div>
          </template>
        </el-table-column>
        <el-table-column label="耗时" width="100">
          <template #default="{ row }">
            <span>{{ row.durationMs ? `${row.durationMs} ms` : '-' }}</span>
          </template>
        </el-table-column>
        <el-table-column label="时间" width="180">
          <template #default="{ row }">
            <span>{{ formatTime(row.finishedAt || row.startedAt) }}</span>
          </template>
        </el-table-column>
        <el-table-column label="操作" width="120" fixed="right">
          <template #default="{ row }">
            <el-button
              type="primary"
              :icon="RefreshRight"
              :disabled="!canRerun(row)"
              @click="prepareRerun(row)"
            >
              重新执行
            </el-button>
          </template>
        </el-table-column>
      </el-table>
    </section>

    <el-dialog v-model="argumentDialogVisible" title="重新执行参数" width="520px">
      <el-form label-position="top">
        <el-form-item v-for="parameter in pendingParameters" :key="parameter.name" :label="parameterLabel(parameter)">
          <el-input v-model="argumentValues[parameter.name]" :placeholder="parameter.defaultValue || ''" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="argumentDialogVisible = false">取消</el-button>
        <el-button type="primary" :icon="RefreshRight" @click="executePendingRerun">重新执行</el-button>
      </template>
    </el-dialog>

    <ResultDetailsDialog v-model="resultDialogVisible" :record="selectedResultRecord" />
  </div>
</template>

<script setup>
import { computed, ref } from 'vue';
import { ElMessage } from 'element-plus/es/components/message/index.mjs';
import { Refresh, RefreshRight, View } from '@element-plus/icons-vue';

import ResultDetailsDialog from '../components/ResultDetailsDialog.vue';
import { useQaStore } from '../composables/useQaStore';
import { formatOutputPreview, formatTime, historyStatusType, parameterLabel } from '../utils/qaFormatters';

const { clients, controller, executeMethod: sendExecute, history, refreshAll } = useQaStore();

const argumentDialogVisible = ref(false);
const argumentValues = ref({});
const pendingMethod = ref(null);
const pendingClientId = ref('');
const resultDialogVisible = ref(false);
const selectedResultRecord = ref(null);
const pendingParameters = computed(() => pendingMethod.value?.parameters || []);

const successCount = computed(() => history.value.filter((item) => item.status === 'success').length);
const failedCount = computed(() => history.value.filter((item) => item.status === 'failed').length);

function canRerun(row) {
  const client = findClient(row.clientId);
  if (!client || client.available === false || client.running || client.clientBusy) {
    return false;
  }

  const lock = client.lock;
  if (lock && lock.ownerId !== controller.value.ownerId) {
    return false;
  }

  return Boolean(findMethod(client, row));
}

function prepareRerun(row) {
  const client = findClient(row.clientId);
  const method = client ? findMethod(client, row) : null;
  if (!client || !method) {
    ElMessage.error('未找到可重新执行的在线方法');
    return;
  }

  pendingClientId.value = client.clientId;
  pendingMethod.value = method;
  argumentValues.value = {};

  const previousArguments = Array.isArray(row.arguments) ? row.arguments : [];
  for (const [index, parameter] of (method.parameters || []).entries()) {
    argumentValues.value[parameter.name] = previousArguments[index] ?? parameter.defaultValue ?? '';
  }

  if ((method.parameters || []).length === 0) {
    executeRerun(client.clientId, method, []);
    return;
  }

  argumentDialogVisible.value = true;
}

function executePendingRerun() {
  const args = pendingParameters.value.map((parameter) => argumentValues.value[parameter.name] ?? '');
  executeRerun(pendingClientId.value, pendingMethod.value, args);
  argumentDialogVisible.value = false;
}

async function executeRerun(clientId, method, args) {
  try {
    await sendExecute(clientId, method, args);
  } catch (error) {
    ElMessage.error(error.message || '重新执行失败');
  }
}

function openResultDetails(row) {
  selectedResultRecord.value = row;
  resultDialogVisible.value = true;
}

function findClient(clientId) {
  return clients.value.find((client) => client.clientId === clientId);
}

function findMethod(client, row) {
  return (client.methods || []).find((method) => method.id === row.methodId || method.name === row.methodName);
}
</script>

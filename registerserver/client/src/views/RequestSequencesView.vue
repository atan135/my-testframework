<template>
  <div class="page-view">
    <header class="topbar">
      <div>
        <h2>请求序列</h2>
      </div>
      <div class="topbar-actions">
        <el-button :icon="Refresh" @click="refreshAll">刷新</el-button>
        <el-button type="primary" :icon="VideoPlay" :disabled="!canRunSequence" @click="executeSequence">执行序列</el-button>
      </div>
    </header>

    <section v-if="selectedClient" class="sequence-layout">
      <section class="section method-catalog">
        <div class="section-heading">
          <div>
            <h3>请求来源</h3>
          </div>
        </div>

        <el-select v-model="selectedClientId" class="full-width" placeholder="选择 Unity 实例">
          <el-option
            v-for="client in clients"
            :key="client.clientId"
            :label="client.name"
            :value="client.clientId"
          />
        </el-select>

        <el-table class="method-table" :data="selectedClient.methods || []" height="360" empty-text="没有可添加的方法">
          <el-table-column prop="name" label="方法" min-width="160" show-overflow-tooltip />
          <el-table-column label="参数" min-width="180" show-overflow-tooltip>
            <template #default="{ row }">
              <span class="muted">{{ formatParameters(row.parameters) }}</span>
            </template>
          </el-table-column>
          <el-table-column label="操作" width="92" fixed="right">
            <template #default="{ row }">
              <el-button type="primary" :icon="Plus" circle @click="addStep(row)" />
            </template>
          </el-table-column>
        </el-table>
      </section>

      <section class="section sequence-builder">
        <div class="section-heading">
          <div>
            <h3>执行步骤</h3>
          </div>
          <div class="sequence-options">
            <label class="sequence-delay-field">
              <span>下步间隔(ms)</span>
              <el-input-number
                v-model="stepDelayMs"
                :min="0"
                :max="300000"
                :step="100"
                :precision="0"
                controls-position="right"
              />
            </label>
            <el-switch v-model="stopOnFailure" active-text="失败即停" />
          </div>
        </div>

        <div v-if="sequenceSteps.length > 0" class="sequence-step-list">
          <article v-for="(step, index) in sequenceSteps" :key="step.stepId" class="sequence-step">
            <div class="sequence-step-header">
              <div>
                <span class="step-index">{{ index + 1 }}</span>
                <strong>{{ step.methodName }}</strong>
                <span class="muted">{{ step.declaringType }}</span>
              </div>
              <div class="step-actions">
                <el-button :icon="ArrowUp" circle :disabled="index === 0" @click="moveStep(index, -1)" />
                <el-button :icon="ArrowDown" circle :disabled="index === sequenceSteps.length - 1" @click="moveStep(index, 1)" />
                <el-button :icon="Delete" circle type="danger" @click="removeStep(index)" />
              </div>
            </div>

            <div v-if="step.parameters.length > 0" class="sequence-parameter-grid">
              <el-input
                v-for="(parameter, parameterIndex) in step.parameters"
                :key="parameter.name"
                v-model="step.arguments[parameterIndex]"
                :placeholder="parameter.defaultValue || ''"
              >
                <template #prepend>{{ parameterLabel(parameter) }}</template>
              </el-input>
            </div>
            <p v-else class="muted no-parameters">无参数</p>
          </article>
        </div>

        <el-empty v-else description="从左侧方法列表添加请求" :image-size="80" />
      </section>
    </section>

    <el-empty v-else class="empty-state" description="启动 Unity 后可以创建请求序列" />

    <section class="section">
      <div class="section-heading">
        <div>
          <h3>服务端返回结果</h3>
        </div>
      </div>

      <el-table :data="sequences" height="430" empty-text="暂无请求序列结果">
        <el-table-column type="expand">
          <template #default="{ row }">
            <el-table :data="row.steps || []" class="sequence-result-table" empty-text="暂无步骤结果">
              <el-table-column label="#" width="70">
                <template #default="{ row: step }">{{ step.stepNumber || step.stepIndex + 1 }}</template>
              </el-table-column>
              <el-table-column prop="status" label="状态" width="110">
                <template #default="{ row: step }">
                  <el-tag :type="historyStatusType(step.status)" effect="plain">{{ step.status || 'pending' }}</el-tag>
                </template>
              </el-table-column>
              <el-table-column prop="methodName" label="方法" min-width="180" show-overflow-tooltip />
              <el-table-column label="输出" min-width="320">
                <template #default="{ row: step }">
                  <div class="result-cell">
                    <span class="result-preview">{{ formatOutputPreview(step) }}</span>
                    <el-button type="primary" link :icon="View" @click="openResultDetails(step)">查看</el-button>
                  </div>
                </template>
              </el-table-column>
              <el-table-column label="耗时" width="110">
                <template #default="{ row: step }">
                  <span>{{ step.durationMs ? `${step.durationMs} ms` : '-' }}</span>
                </template>
              </el-table-column>
            </el-table>
          </template>
        </el-table-column>
        <el-table-column prop="status" label="状态" width="110">
          <template #default="{ row }">
            <el-tag :type="historyStatusType(row.status)" effect="plain">{{ row.status }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column prop="sequenceId" label="序列 ID" min-width="240" show-overflow-tooltip />
        <el-table-column label="进度" width="120">
          <template #default="{ row }">{{ row.completedSteps || 0 }} / {{ row.totalSteps || 0 }}</template>
        </el-table-column>
        <el-table-column label="间隔" width="110">
          <template #default="{ row }">{{ row.stepDelayMs || 0 }} ms</template>
        </el-table-column>
        <el-table-column label="成功" width="90">
          <template #default="{ row }">{{ row.successCount || 0 }}</template>
        </el-table-column>
        <el-table-column label="失败" width="90">
          <template #default="{ row }">{{ row.failedCount || 0 }}</template>
        </el-table-column>
        <el-table-column label="时间" width="180">
          <template #default="{ row }">
            <span>{{ formatTime(row.finishedAt || row.startedAt) }}</span>
          </template>
        </el-table-column>
        <el-table-column label="操作" width="110" fixed="right">
          <template #default="{ row }">
            <el-button
              v-if="row.status === 'running'"
              type="danger"
              :icon="CircleClose"
              @click="stopRunningSequence(row.sequenceId)"
            >
              停止
            </el-button>
          </template>
        </el-table-column>
      </el-table>
    </section>

    <ResultDetailsDialog v-model="resultDialogVisible" :record="selectedResultRecord" />
  </div>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import { ElMessage } from 'element-plus/es/components/message/index.mjs';
import { ArrowDown, ArrowUp, CircleClose, Delete, Plus, Refresh, VideoPlay, View } from '@element-plus/icons-vue';

import ResultDetailsDialog from '../components/ResultDetailsDialog.vue';
import { useQaStore } from '../composables/useQaStore';
import { formatOutputPreview, formatParameters, formatTime, historyStatusType, parameterLabel } from '../utils/qaFormatters';

const { clients, controller, refreshAll, runSequence, selectedClient, selectedClientId, sequences, stopSequence: sendStopSequence, wsStatus } = useQaStore();

const sequenceSteps = ref([]);
const stepDelayMs = ref(0);
const stopOnFailure = ref(true);
const resultDialogVisible = ref(false);
const selectedResultRecord = ref(null);

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
const canRunSequence = computed(() =>
  canControlSelectedClient.value &&
  sequenceSteps.value.length > 0 &&
  wsStatus.value === 'connected'
);

watch(selectedClientId, () => {
  sequenceSteps.value = [];
});

function addStep(method) {
  sequenceSteps.value.push({
    stepId: createLocalId(),
    methodId: method.id,
    methodName: method.name,
    declaringType: method.declaringType,
    parameters: method.parameters || [],
    arguments: (method.parameters || []).map((parameter) => parameter.defaultValue || ''),
  });
}

function removeStep(index) {
  sequenceSteps.value.splice(index, 1);
}

function moveStep(index, offset) {
  const nextIndex = index + offset;
  if (nextIndex < 0 || nextIndex >= sequenceSteps.value.length) {
    return;
  }

  const [step] = sequenceSteps.value.splice(index, 1);
  sequenceSteps.value.splice(nextIndex, 0, step);
}

function executeSequence() {
  try {
    const sequenceId = runSequence({
      clientId: selectedClient.value.clientId,
      stepDelayMs: stepDelayMs.value,
      stopOnFailure: stopOnFailure.value,
      steps: sequenceSteps.value.map((step) => ({
        stepId: step.stepId,
        methodId: step.methodId,
        methodName: step.methodName,
        arguments: step.arguments,
      })),
    });
    ElMessage.success(`请求序列已提交：${sequenceId}`);
  } catch (error) {
    ElMessage.error(error.message || '请求序列提交失败');
  }
}

function stopRunningSequence(sequenceId) {
  try {
    sendStopSequence(sequenceId);
  } catch (error) {
    ElMessage.error(error.message || '停止请求提交失败');
  }
}

function openResultDetails(row) {
  selectedResultRecord.value = row;
  resultDialogVisible.value = true;
}

function createLocalId() {
  if (window.crypto?.randomUUID) {
    return window.crypto.randomUUID();
  }
  return `${Date.now()}-${Math.random().toString(16).slice(2)}`;
}
</script>

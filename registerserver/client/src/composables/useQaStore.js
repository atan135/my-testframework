import { computed, ref } from 'vue';
import { ElMessage } from 'element-plus/es/components/message/index.mjs';

const clients = ref([]);
const history = ref([]);
const sequences = ref([]);
const selectedClientId = ref('');
const wsStatus = ref('disconnected');
const controller = ref({
  ownerId: getControllerId(),
  ownerType: 'web',
});

let socket;
let reconnectTimer;
let started = false;

const selectedClient = computed(() => clients.value.find((client) => client.clientId === selectedClientId.value));

export function useQaStore() {
  return {
    clients,
    controller,
    executeMethod,
    history,
    refreshAll,
    runSequence,
    selectedClient,
    selectedClientId,
    sequences,
    start,
    stop,
    stopSequence,
    wsStatus,
  };
}

function start() {
  if (started) {
    return;
  }

  started = true;
  refreshAll();
  connectWebSocket();
}

function stop() {
  started = false;
  window.clearTimeout(reconnectTimer);
  reconnectTimer = null;
  socket?.close();
  socket = null;
}

async function refreshAll() {
  try {
    const [clientPayload, resultPayload] = await Promise.all([
      requestJson('/api/unity-clients'),
      requestJson('/api/results'),
    ]);

    clients.value = clientPayload.clients || [];
    history.value = resultPayload.results || [];
    ensureSelectedClient();
  } catch (error) {
    ElMessage.error(error.message || '刷新失败');
  }
}

async function executeMethod(clientId, method, args) {
  if (!clientId || !method) {
    return;
  }

  sendSocketMessage({
    type: 'execute',
    clientId,
    methodId: method.id,
    methodName: method.name,
    arguments: args,
  });
}

function runSequence({ clientId, steps, stopOnFailure, stepDelayMs }) {
  if (!clientId) {
    throw new Error('请选择 Unity 实例');
  }

  if (!Array.isArray(steps) || steps.length === 0) {
    throw new Error('请至少添加一个请求');
  }

  const sequenceId = createId();
  sendSocketMessage({
    type: 'execute_sequence',
    sequenceId,
    clientId,
    stepDelayMs: normalizeStepDelayMs(stepDelayMs),
    stopOnFailure,
    steps,
  });
  return sequenceId;
}

function stopSequence(sequenceId, reason = 'Stopped from web console.') {
  if (!sequenceId) {
    throw new Error('缺少请求序列 ID');
  }

  sendSocketMessage({
    type: 'stop_sequence',
    sequenceId,
    reason,
  });
}

function connectWebSocket() {
  window.clearTimeout(reconnectTimer);
  wsStatus.value = 'connecting';

  const protocol = window.location.protocol === 'https:' ? 'wss' : 'ws';
  const query = new URLSearchParams({
    role: 'web',
    controllerType: 'web',
    controllerId: controller.value.ownerId,
  });
  socket = new WebSocket(`${protocol}://${window.location.host}/ws?${query.toString()}`);

  socket.addEventListener('open', () => {
    wsStatus.value = 'connected';
  });

  socket.addEventListener('message', (event) => {
    const message = JSON.parse(event.data);
    handleSocketMessage(message);
  });

  socket.addEventListener('close', () => {
    wsStatus.value = 'disconnected';
    if (started) {
      reconnectTimer = window.setTimeout(connectWebSocket, 2000);
    }
  });

  socket.addEventListener('error', () => {
    wsStatus.value = 'disconnected';
  });
}

function handleSocketMessage(message) {
  if (message.type === 'snapshot') {
    if (message.controller) {
      controller.value = message.controller;
    }
    clients.value = message.clients || [];
    history.value = message.history || [];
    ensureSelectedClient();
    return;
  }

  if (message.type === 'unity_registered' || message.type === 'unity_state_changed') {
    upsertClient(message.client);
    ensureSelectedClient();
    return;
  }

  if (message.type === 'unity_available' || message.type === 'unity_unavailable') {
    upsertClient(message.client);
    ensureSelectedClient();
    if (message.type === 'unity_unavailable') {
      ElMessage.warning(`Unity 不可用：${message.client?.name || message.client?.clientId || ''}`);
    }
    return;
  }

  if (message.type === 'unity_disconnected') {
    clients.value = clients.value.filter((client) => client.clientId !== message.clientId);
    ensureSelectedClient();
    ElMessage.warning(`Unity 已断开：${message.clientId}`);
    return;
  }

  if (message.type === 'execution_started') {
    patchClient(message.execution?.clientId, { running: true });
    prependHistory(message.execution);
    return;
  }

  if (message.type === 'qa_result') {
    if (!message.result?.sequenceId) {
      patchClient(message.result?.clientId, { running: false });
    }
    prependHistory(message.result);
    return;
  }

  if (message.type === 'execute_accepted') {
    prependHistory(message.execution);
    return;
  }

  if (message.type === 'execute_rejected') {
    ElMessage.error(message.error || '执行请求被拒绝');
    return;
  }

  if (message.type === 'client_locked') {
    patchClient(message.clientId, { lock: message.lock });
    return;
  }

  if (message.type === 'client_unlocked') {
    patchClient(message.clientId, { lock: null });
    return;
  }

  if (message.type === 'stop_accepted') {
    ElMessage.success('停止请求已提交');
    return;
  }

  if (message.type === 'stop_rejected') {
    ElMessage.error(message.error || '停止请求被拒绝');
    return;
  }

  if (message.type === 'sequence_started') {
    patchClient(message.sequence?.clientId, { running: true });
    upsertSequence(message.sequence);
    return;
  }

  if (message.type === 'sequence_step_started') {
    markSequenceStepStarted(message.sequenceId, message.step, message.execution);
    return;
  }

  if (message.type === 'sequence_step_result') {
    markSequenceStepResult(message.sequenceId, message.step, message.result);
    return;
  }

  if (message.type === 'sequence_finished') {
    patchClient(message.sequence?.clientId, { running: false });
    upsertSequence(message.sequence);
    return;
  }

  if (message.type === 'error') {
    ElMessage.error(message.error || '服务端错误');
  }
}

async function requestJson(url) {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`Request failed: ${response.status}`);
  }
  return response.json();
}

function upsertClient(client) {
  const index = clients.value.findIndex((item) => item.clientId === client.clientId);
  if (index >= 0) {
    clients.value.splice(index, 1, client);
    return;
  }
  clients.value.unshift(client);
}

function patchClient(clientId, patch) {
  const index = clients.value.findIndex((item) => item.clientId === clientId);
  if (index >= 0) {
    clients.value.splice(index, 1, {
      ...clients.value[index],
      ...patch,
    });
  }
}

function prependHistory(item) {
  if (item.requestId) {
    const existingIndex = history.value.findIndex((row) => row.requestId === item.requestId);
    if (existingIndex >= 0) {
      history.value.splice(existingIndex, 1, { ...history.value[existingIndex], ...item });
      return;
    }
  }
  history.value = [item, ...history.value].slice(0, 200);
}

function upsertSequence(sequence) {
  const normalized = {
    ...sequence,
    steps: sequence.steps || [],
    results: sequence.results || [],
  };
  const index = sequences.value.findIndex((item) => item.sequenceId === normalized.sequenceId);
  if (index >= 0) {
    const existing = sequences.value[index];
    sequences.value.splice(index, 1, {
      ...existing,
      ...normalized,
      steps: mergeSequenceSteps(existing.steps || [], normalized.steps || []),
      results: normalized.results.length > 0 ? normalized.results : existing.results || [],
    });
    return;
  }
  sequences.value = [normalized, ...sequences.value].slice(0, 50);
}

function markSequenceStepStarted(sequenceId, step, execution) {
  const sequence = ensureSequence(sequenceId);
  upsertSequenceStep(sequence, {
    ...step,
    ...execution,
    status: 'running',
  });
}

function markSequenceStepResult(sequenceId, step, result) {
  const sequence = ensureSequence(sequenceId);
  upsertSequenceStep(sequence, {
    ...step,
    ...result,
  });

  const resultIndex = sequence.results.findIndex((item) => item.requestId === result.requestId);
  if (resultIndex >= 0) {
    sequence.results.splice(resultIndex, 1, result);
  } else {
    sequence.results.push(result);
  }

  sequence.completedSteps = sequence.steps.filter((item) => item.status === 'success' || item.status === 'failed').length;
  sequence.successCount = sequence.steps.filter((item) => item.status === 'success').length;
  sequence.failedCount = sequence.steps.filter((item) => item.status === 'failed').length;
  if (sequence.failedCount > 0) {
    sequence.status = 'failed';
  }
}

function ensureSequence(sequenceId) {
  let sequence = sequences.value.find((item) => item.sequenceId === sequenceId);
  if (!sequence) {
    sequence = {
      sequenceId,
      status: 'running',
      steps: [],
      results: [],
      totalSteps: 0,
      completedSteps: 0,
      successCount: 0,
      failedCount: 0,
      startedAt: new Date().toISOString(),
    };
    sequences.value.unshift(sequence);
  }
  return sequence;
}

function upsertSequenceStep(sequence, step) {
  const index = sequence.steps.findIndex((item) => item.stepId === step.stepId);
  if (index >= 0) {
    sequence.steps.splice(index, 1, {
      ...sequence.steps[index],
      ...step,
    });
    return;
  }
  sequence.steps.push(step);
}

function mergeSequenceSteps(existingSteps, incomingSteps) {
  const mergedSteps = [...existingSteps];
  for (const incomingStep of incomingSteps) {
    const index = mergedSteps.findIndex((step) => step.stepId === incomingStep.stepId);
    if (index >= 0) {
      mergedSteps.splice(index, 1, {
        ...incomingStep,
        ...mergedSteps[index],
      });
    } else {
      mergedSteps.push(incomingStep);
    }
  }
  return mergedSteps.sort((a, b) => (a.stepIndex || 0) - (b.stepIndex || 0));
}

function ensureSelectedClient() {
  if (clients.value.length === 0) {
    selectedClientId.value = '';
    return;
  }
  if (!clients.value.some((client) => client.clientId === selectedClientId.value)) {
    selectedClientId.value = clients.value[0].clientId;
  }
}

function sendSocketMessage(message) {
  if (!socket || socket.readyState !== WebSocket.OPEN) {
    throw new Error('WebSocket 未连接');
  }
  socket.send(JSON.stringify(message));
}

function createId() {
  if (window.crypto?.randomUUID) {
    return window.crypto.randomUUID();
  }
  return `${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function normalizeStepDelayMs(value) {
  const delay = Number(value || 0);
  if (!Number.isFinite(delay)) {
    return 0;
  }
  return Math.min(Math.max(Math.floor(delay), 0), 300000);
}

function getControllerId() {
  const key = 'QaTest.ControllerId';
  const existing = window.sessionStorage.getItem(key);
  if (existing) {
    return existing;
  }

  const next = window.crypto?.randomUUID ? window.crypto.randomUUID() : `${Date.now()}-${Math.random().toString(16).slice(2)}`;
  window.sessionStorage.setItem(key, next);
  return next;
}

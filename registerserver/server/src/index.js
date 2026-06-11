import crypto from 'node:crypto';
import http from 'node:http';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import cors from 'cors';
import express from 'express';
import WebSocket, { WebSocketServer } from 'ws';

const PORT = toPositiveInteger(process.env.PORT, 3000);
const HEARTBEAT_INTERVAL_MS = toPositiveInteger(process.env.WS_HEARTBEAT_INTERVAL_MS, 15000);
const UNITY_HEARTBEAT_STALE_MS = toPositiveInteger(process.env.UNITY_HEARTBEAT_STALE_MS, 45000);
const HISTORY_LIMIT = 200;
const EXECUTION_TIMEOUT_MS = toPositiveInteger(process.env.EXECUTION_TIMEOUT_MS, 20000);
const MAX_SEQUENCE_STEP_DELAY_MS = 300000;

const app = express();
const server = http.createServer(app);
const wss = new WebSocketServer({ server, path: '/ws' });

const unityClients = new Map();
const webClients = new Set();
const controllers = new Map();
const executionHistory = [];
const pendingExecutions = new Map();
const activeSequences = new Map();

app.use(cors());
app.use(express.json({ limit: '1mb' }));

app.get('/api/health', (_req, res) => {
  res.json({
    ok: true,
    uptime: process.uptime(),
    unityClientCount: unityClients.size,
    webClientCount: webClients.size,
    controllerCount: controllers.size,
    executionTimeoutMs: EXECUTION_TIMEOUT_MS,
    unityHeartbeatStaleMs: UNITY_HEARTBEAT_STALE_MS,
  });
});

app.get('/api/unity-clients', (_req, res) => {
  res.json({ clients: getUnityClientSnapshot() });
});

app.get('/api/results', (_req, res) => {
  res.json({ results: executionHistory });
});

app.post('/api/unity-clients/:clientId/execute', (req, res) => {
  const { clientId } = req.params;
  const { methodId, methodName, arguments: methodArguments } = req.body || {};
  const controller = createTransientController('http');

  try {
    const { started } = dispatchUnityExecution({
      clientId,
      methodId,
      methodName,
      methodArguments,
      controller,
    });

    res.status(202).json(started);
  } catch (error) {
    const status = error.statusCode || 500;
    res.status(status).json({ error: error.message });
  }
});

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const clientDistPath = path.resolve(__dirname, '../../client/dist');
app.use(express.static(clientDistPath));
app.get('*', (req, res, next) => {
  if (req.path.startsWith('/api/')) {
    next();
    return;
  }
  res.sendFile(path.join(clientDistPath, 'index.html'), (error) => {
    if (error) {
      next();
    }
  });
});

wss.on('connection', (socket, req) => {
  const url = new URL(req.url || '/', `http://${req.headers.host || 'localhost'}`);
  const role = url.searchParams.get('role');

  socket.isAlive = true;
  socket.role = role === 'unity' ? 'unity' : 'web';
  socket.on('pong', () => {
    socket.isAlive = true;
  });

  if (role === 'unity') {
    handleUnityConnection(socket);
    return;
  }

  handleWebConnection(socket, url);
});

server.listen(PORT, () => {
  console.log(`QA register server listening on http://localhost:${PORT}`);
});

setInterval(() => {
  for (const socket of wss.clients) {
    if (!socket.isAlive) {
      handleSocketProtocolFailure(socket, 'ping_timeout');
      socket.terminate();
      continue;
    }

    socket.isAlive = false;
    try {
      socket.ping();
    } catch {
      handleSocketProtocolFailure(socket, 'ping_error');
      socket.terminate();
    }
  }

  const now = Date.now();
  for (const client of unityClients.values()) {
    if (client.available && now - client.lastSeenAt > UNITY_HEARTBEAT_STALE_MS) {
      markUnityUnavailable(client, 'heartbeat_timeout');
    }
  }
}, HEARTBEAT_INTERVAL_MS);

function handleUnityConnection(socket) {
  let boundClientId = null;

  socket.on('message', (payload) => {
    const message = parseMessage(payload);
    if (!message) {
      send(socket, { type: 'error', error: 'Invalid JSON message.' });
      return;
    }

    if (message.type === 'register') {
      boundClientId = registerUnityClient(socket, message);
      socket.boundClientId = boundClientId;
      send(socket, { type: 'registered', clientId: boundClientId });
      broadcastWeb({ type: 'unity_registered', client: toPublicClient(unityClients.get(boundClientId)) });
      return;
    }

    if (!boundClientId && message.clientId) {
      boundClientId = message.clientId;
      socket.boundClientId = boundClientId;
    }

    const client = getUnityClientForSocket(boundClientId, socket);
    if (client) {
      touchUnityClient(client, message.type || 'message');
    }

    if (message.type === 'heartbeat') {
      send(socket, { type: 'heartbeat_ack', serverTime: new Date().toISOString() });
      return;
    }

    if (message.type === 'qa_result') {
      handleUnityQaResult(message, boundClientId);
    }
  });

  socket.on('close', () => {
    if (boundClientId) {
      removeUnityClient(boundClientId, 'closed', socket);
    }
  });

  socket.on('error', () => {
    if (boundClientId) {
      removeUnityClient(boundClientId, 'error', socket);
    }
  });
}

function handleWebConnection(socket, url) {
  const controller = attachController(socket, url);
  webClients.add(socket);

  send(socket, {
    type: 'snapshot',
    controller: toPublicController(controller),
    clients: getUnityClientSnapshot(),
    history: executionHistory,
  });

  socket.on('message', (payload) => {
    const message = parseMessage(payload);
    if (!message) {
      send(socket, { type: 'error', error: 'Invalid JSON message.' });
      return;
    }

    controller.lastSeenAt = Date.now();

    if (message.type === 'refresh') {
      send(socket, {
        type: 'snapshot',
        controller: toPublicController(controller),
        clients: getUnityClientSnapshot(),
        history: executionHistory,
      });
      return;
    }

    if (message.type === 'execute') {
      handleWebExecute(socket, controller, message);
      return;
    }

    if (message.type === 'execute_sequence') {
      runExecutionSequence(controller, message).catch((error) => {
        send(socket, { type: 'error', error: error.message || 'Failed to execute sequence.' });
      });
      return;
    }

    if (message.type === 'stop_sequence' || message.type === 'cancel_sequence' || message.type === 'stop_execution' || message.type === 'cancel_execution' || message.type === 'stop') {
      handleWebStop(socket, controller, message);
    }
  });

  socket.on('close', () => {
    webClients.delete(socket);
    detachController(socket, 'closed');
  });

  socket.on('error', () => {
    webClients.delete(socket);
    detachController(socket, 'error');
  });
}

function handleWebExecute(socket, controller, message) {
  try {
    const { started } = dispatchUnityExecution({
      clientId: message.clientId,
      methodId: message.methodId,
      methodName: message.methodName,
      methodArguments: message.arguments,
      controller,
    });

    send(socket, { type: 'execute_accepted', execution: started });
  } catch (error) {
    send(socket, {
      type: 'execute_rejected',
      error: error.message || 'Failed to execute request.',
    });
  }
}

function handleWebStop(socket, controller, message) {
  try {
    const stopped = stopExecutionOrSequence({
      requestId: message.requestId,
      sequenceId: message.sequenceId,
      controller,
      reason: message.reason || 'Stopped by controller.',
    });
    send(socket, { type: 'stop_accepted', ...stopped });
  } catch (error) {
    send(socket, {
      type: 'stop_rejected',
      error: error.message || 'Failed to stop execution.',
    });
  }
}

async function runExecutionSequence(controller, message) {
  const sequenceId = message.sequenceId || crypto.randomUUID();
  const clientId = message.clientId;
  const steps = normalizeSequenceSteps(message.steps);
  const stopOnFailure = message.stopOnFailure !== false;
  const stepDelayMs = normalizeDelayMs(message.stepDelayMs, MAX_SEQUENCE_STEP_DELAY_MS);
  const startedAt = new Date().toISOString();

  if (!clientId) {
    throw createRequestError('clientId is required.', 400);
  }

  if (steps.length === 0) {
    throw createRequestError('At least one request step is required.', 400);
  }

  prepareClientForExecution({
    clientId,
    controller,
    allowBusy: false,
  });

  const sequence = {
    sequenceId,
    clientId,
    status: 'running',
    stopOnFailure,
    stepDelayMs,
    totalSteps: steps.length,
    completedSteps: 0,
    successCount: 0,
    failedCount: 0,
    cancelledCount: 0,
    startedAt,
    steps: steps.map((step, index) => toPublicSequenceStep(step, index, steps.length)),
    results: [],
  };

  const sequenceState = {
    sequenceId,
    clientId,
    ownerId: controller.id,
    ownerType: controller.type,
    status: 'running',
    cancelled: false,
    cancelReason: '',
    currentRequestId: '',
    waiters: new Set(),
  };

  activeSequences.set(sequenceId, sequenceState);
  broadcastWeb({ type: 'sequence_started', sequence });

  try {
    for (let index = 0; index < steps.length; index++) {
      if (sequenceState.cancelled) {
        break;
      }

      const step = steps[index];
      const stepMeta = {
        sequenceId,
        stepId: step.stepId,
        stepIndex: index,
        stepNumber: index + 1,
        totalSteps: steps.length,
      };

      if (index > 0 && stepDelayMs > 0) {
        await sleep(stepDelayMs, sequenceState);
        if (sequenceState.cancelled) {
          break;
        }
      }

      let result;
      try {
        const { started, resultPromise } = dispatchUnityExecution({
          clientId,
          methodId: step.methodId,
          methodName: step.methodName,
          methodArguments: step.arguments,
          controller,
          waitForResult: true,
          allowBusy: true,
          meta: stepMeta,
        });

        sequenceState.currentRequestId = started.requestId;
        broadcastWeb({
          type: 'sequence_step_started',
          sequenceId,
          step: toPublicSequenceStep(step, index, steps.length),
          execution: started,
        });

        result = await resultPromise;
      } catch (error) {
        result = buildServerFailureResult({
          clientId,
          methodId: step.methodId,
          methodName: step.methodName,
          methodArguments: step.arguments,
          error,
          meta: stepMeta,
        });
        addHistory(result);
        broadcastWeb({ type: 'qa_result', result });
      } finally {
        sequenceState.currentRequestId = '';
      }

      sequence.completedSteps += 1;
      if (result.status === 'cancelled') {
        sequence.cancelledCount += 1;
      } else if (result.success) {
        sequence.successCount += 1;
      } else {
        sequence.failedCount += 1;
      }
      sequence.results.push(result);

      broadcastWeb({
        type: 'sequence_step_result',
        sequenceId,
        step: toPublicSequenceStep(step, index, steps.length),
        result,
      });

      if (sequenceState.cancelled || result.status === 'cancelled') {
        sequenceState.cancelled = true;
        break;
      }

      if (stopOnFailure && !result.success) {
        break;
      }
    }

    if (sequenceState.cancelled) {
      sequence.status = 'cancelled';
      sequence.cancelReason = sequenceState.cancelReason || 'Stopped by controller.';
    } else {
      sequence.status = sequence.failedCount > 0 ? 'failed' : 'success';
    }
    sequence.finishedAt = new Date().toISOString();
    broadcastWeb({ type: 'sequence_finished', sequence });
  } finally {
    activeSequences.delete(sequenceId);
    releaseClientLockIfIdle(clientId, 'sequence_finished');
  }
}

function dispatchUnityExecution({
  clientId,
  methodId,
  methodName,
  methodArguments,
  controller,
  waitForResult = false,
  allowBusy = false,
  meta = {},
}) {
  const resolvedMethodId = methodId || methodName;
  if (!resolvedMethodId) {
    throw createRequestError('methodId is required.', 400);
  }

  const client = prepareClientForExecution({
    clientId,
    controller,
    allowBusy,
  });

  const requestId = crypto.randomUUID();
  const command = {
    type: 'execute',
    requestId,
    methodId: resolvedMethodId,
    methodName: methodName || resolvedMethodId,
    arguments: normalizeArguments(methodArguments),
  };

  const started = {
    requestId,
    clientId,
    methodId: resolvedMethodId,
    methodName: command.methodName,
    arguments: command.arguments,
    status: 'running',
    startedAt: new Date().toISOString(),
    ...meta,
  };

  const resultPromise = createPendingExecution(started, controller);
  addHistory(started);
  broadcastWeb({ type: 'execution_started', execution: started });

  try {
    client.socket.send(JSON.stringify(command), (error) => {
      if (error) {
        completePendingExecution(requestId, {
          status: 'failed',
          success: false,
          result: '',
          error: error.message || 'Failed to send execution command.',
          durationMs: 0,
          finishedAt: new Date().toISOString(),
        });
      }
    });
  } catch (error) {
    completePendingExecution(requestId, {
      status: 'failed',
      success: false,
      result: '',
      error: error.message || 'Failed to send execution command.',
      durationMs: 0,
      finishedAt: new Date().toISOString(),
    });
  }

  return { requestId, started, resultPromise: waitForResult ? resultPromise : null };
}

function createPendingExecution(started, controller) {
  return new Promise((resolve) => {
    const timeout = setTimeout(() => {
      completePendingExecution(started.requestId, {
        status: 'failed',
        success: false,
        result: '',
        error: `Timed out after ${EXECUTION_TIMEOUT_MS} ms.`,
        durationMs: EXECUTION_TIMEOUT_MS,
        finishedAt: new Date().toISOString(),
      });
    }, EXECUTION_TIMEOUT_MS);

    pendingExecutions.set(started.requestId, {
      started,
      timeout,
      resolve,
      clientId: started.clientId,
      ownerId: controller.id,
      ownerType: controller.type,
    });
  });
}

function completePendingExecution(requestId, patch) {
  const pending = pendingExecutions.get(requestId);
  if (!pending) {
    return null;
  }

  clearTimeout(pending.timeout);
  pendingExecutions.delete(requestId);

  const result = {
    ...pending.started,
    ...patch,
    requestId,
    clientId: patch.clientId || pending.started.clientId,
    methodId: patch.methodId || pending.started.methodId,
    methodName: patch.methodName || pending.started.methodName,
  };

  addHistory(result);
  broadcastWeb({ type: 'qa_result', result });
  pending.resolve(result);
  releaseClientLockIfIdle(result.clientId, 'execution_finished');
  return result;
}

function handleUnityQaResult(message, boundClientId) {
  const result = {
    requestId: message.requestId,
    clientId: message.clientId || boundClientId,
    methodId: message.methodId || message.methodName,
    methodName: message.methodName || message.methodId,
    status: message.success ? 'success' : 'failed',
    success: Boolean(message.success),
    result: message.result || '',
    error: message.error || '',
    durationMs: Number(message.durationMs || 0),
    finishedAt: new Date().toISOString(),
  };

  if (pendingExecutions.has(result.requestId)) {
    completePendingExecution(result.requestId, result);
    return;
  }

  const existing = findHistoryByRequestId(result.requestId);
  if (existing && existing.status !== 'running') {
    broadcastWeb({ type: 'qa_result_late', result: { ...result, late: true } });
    return;
  }

  addHistory(result);
  broadcastWeb({ type: 'qa_result', result });
  releaseClientLockIfIdle(result.clientId, 'unexpected_result');
}

function stopExecutionOrSequence({ requestId, sequenceId, controller, reason }) {
  if (sequenceId) {
    const sequenceState = activeSequences.get(sequenceId);
    if (!sequenceState) {
      throw createRequestError(`Sequence ${sequenceId} is not running.`, 404);
    }
    assertOwner(sequenceState.ownerId, controller);
    sequenceState.cancelled = true;
    sequenceState.cancelReason = reason;
    resolveSequenceWaiters(sequenceState);

    if (sequenceState.currentRequestId) {
      completePendingExecution(sequenceState.currentRequestId, {
        status: 'cancelled',
        success: false,
        result: '',
        error: reason,
        durationMs: 0,
        finishedAt: new Date().toISOString(),
      });
    }

    return { sequenceId, status: 'cancelling' };
  }

  if (requestId) {
    const pending = pendingExecutions.get(requestId);
    if (!pending) {
      throw createRequestError(`Execution ${requestId} is not running.`, 404);
    }
    assertOwner(pending.ownerId, controller);

    const sequenceState = pending.started.sequenceId ? activeSequences.get(pending.started.sequenceId) : null;
    if (sequenceState) {
      sequenceState.cancelled = true;
      sequenceState.cancelReason = reason;
      resolveSequenceWaiters(sequenceState);
    }

    completePendingExecution(requestId, {
      status: 'cancelled',
      success: false,
      result: '',
      error: reason,
      durationMs: 0,
      finishedAt: new Date().toISOString(),
    });
    return { requestId, status: 'cancelled' };
  }

  throw createRequestError('requestId or sequenceId is required.', 400);
}

function prepareClientForExecution({ clientId, controller, allowBusy }) {
  const client = unityClients.get(clientId);

  if (!client || !isSocketOpen(client.socket)) {
    throw createRequestError(`Unity client ${clientId} is not online.`, 404);
  }

  if (!client.available) {
    throw createRequestError(`Unity client ${clientId} is unavailable.`, 409);
  }

  if (client.lock && client.lock.ownerId !== controller.id) {
    throw createRequestError(`Unity client ${clientId} is locked by another controller.`, 423);
  }

  if (!allowBusy && isClientRunning(clientId)) {
    throw createRequestError(`Unity client ${clientId} is already running a request.`, 409);
  }

  acquireClientLock(client, controller);
  return client;
}

function acquireClientLock(client, controller) {
  const now = Date.now();
  if (client.lock && client.lock.ownerId === controller.id) {
    client.lock.lastSeenAt = now;
    return;
  }

  client.lock = {
    ownerId: controller.id,
    ownerType: controller.type,
    acquiredAt: now,
    lastSeenAt: now,
  };
  broadcastWeb({ type: 'client_locked', clientId: client.clientId, lock: toPublicLock(client.lock) });
}

function releaseClientLockIfIdle(clientId, reason) {
  const client = unityClients.get(clientId);
  if (!client?.lock || isClientRunning(clientId, client.lock.ownerId)) {
    return;
  }

  const lock = client.lock;
  client.lock = null;
  broadcastWeb({ type: 'client_unlocked', clientId, reason, lock: toPublicLock(lock) });
}

function releaseIdleLocksForController(ownerId, reason) {
  for (const client of unityClients.values()) {
    if (client.lock?.ownerId === ownerId && !isClientRunning(client.clientId, ownerId)) {
      const lock = client.lock;
      client.lock = null;
      broadcastWeb({ type: 'client_unlocked', clientId: client.clientId, reason, lock: toPublicLock(lock) });
    }
  }
}

function isClientRunning(clientId, ownerId = '') {
  for (const pending of pendingExecutions.values()) {
    if (pending.clientId === clientId && (!ownerId || pending.ownerId === ownerId)) {
      return true;
    }
  }

  for (const sequence of activeSequences.values()) {
    if (sequence.clientId === clientId && sequence.status === 'running' && (!ownerId || sequence.ownerId === ownerId)) {
      return true;
    }
  }

  return false;
}

function assertOwner(ownerId, controller) {
  if (ownerId !== controller.id) {
    throw createRequestError('Only the controller that owns this execution can stop it.', 403);
  }
}

function normalizeSequenceSteps(rawSteps) {
  if (!Array.isArray(rawSteps)) {
    return [];
  }

  return rawSteps
    .map((step, index) => ({
      stepId: step.stepId || crypto.randomUUID(),
      methodId: step.methodId || step.methodName,
      methodName: step.methodName || step.methodId || `Step ${index + 1}`,
      arguments: normalizeArguments(step.arguments),
    }))
    .filter((step) => step.methodId);
}

function toPublicSequenceStep(step, index, totalSteps) {
  return {
    stepId: step.stepId,
    stepIndex: index,
    stepNumber: index + 1,
    totalSteps,
    methodId: step.methodId,
    methodName: step.methodName,
    arguments: step.arguments,
  };
}

function buildServerFailureResult({ clientId, methodId, methodName, methodArguments, error, meta }) {
  return {
    requestId: crypto.randomUUID(),
    clientId,
    methodId: methodId || methodName,
    methodName: methodName || methodId,
    arguments: normalizeArguments(methodArguments),
    status: 'failed',
    success: false,
    result: '',
    error: error.message || String(error),
    durationMs: 0,
    finishedAt: new Date().toISOString(),
    ...meta,
  };
}

function normalizeArguments(methodArguments) {
  return Array.isArray(methodArguments) ? methodArguments.map((argument) => String(argument)) : [];
}

function normalizeDelayMs(value, maxValue) {
  const delay = Number(value || 0);
  if (!Number.isFinite(delay)) {
    return 0;
  }
  return Math.min(Math.max(Math.floor(delay), 0), maxValue);
}

function sleep(ms, sequenceState = null) {
  if (!sequenceState) {
    return new Promise((resolve) => {
      setTimeout(resolve, ms);
    });
  }

  return new Promise((resolve) => {
    const timeout = setTimeout(() => {
      sequenceState.waiters.delete(done);
      resolve();
    }, ms);

    function done() {
      clearTimeout(timeout);
      sequenceState.waiters.delete(done);
      resolve();
    }

    sequenceState.waiters.add(done);
  });
}

function resolveSequenceWaiters(sequenceState) {
  for (const waiter of Array.from(sequenceState.waiters)) {
    waiter();
  }
}

function createRequestError(message, statusCode) {
  const error = new Error(message);
  error.statusCode = statusCode;
  return error;
}

function registerUnityClient(socket, message) {
  const clientId = message.clientId || crypto.randomUUID();
  const existing = unityClients.get(clientId);

  if (existing && existing.socket !== socket && isSocketOpen(existing.socket)) {
    existing.socket.close(1000, 'Replaced by a new connection.');
  }

  const now = Date.now();
  const client = {
    clientId,
    name: message.name || clientId,
    platform: message.platform || 'unknown',
    unityVersion: message.unityVersion || '',
    methods: Array.isArray(message.methods) ? message.methods : [],
    connectedAt: existing?.connectedAt || now,
    lastSeenAt: now,
    availabilityChangedAt: existing?.availabilityChangedAt || now,
    available: true,
    unavailableReason: '',
    socket,
    lock: existing?.lock || null,
  };

  unityClients.set(clientId, client);
  return clientId;
}

function getUnityClientForSocket(clientId, socket) {
  const client = clientId ? unityClients.get(clientId) : null;
  if (!client || client.socket !== socket) {
    return null;
  }
  return client;
}

function touchUnityClient(client, source) {
  client.lastSeenAt = Date.now();
  if (!client.available) {
    markUnityAvailable(client, source);
  }
}

function markUnityUnavailable(client, reason) {
  client.available = false;
  client.unavailableReason = reason;
  client.availabilityChangedAt = Date.now();
  broadcastWeb({ type: 'unity_unavailable', client: toPublicClient(client), reason });
}

function markUnityAvailable(client, source) {
  client.available = true;
  client.unavailableReason = '';
  client.availabilityChangedAt = Date.now();
  broadcastWeb({ type: 'unity_available', client: toPublicClient(client), source });
}

function removeUnityClient(clientId, reason, socket = null) {
  const client = unityClients.get(clientId);
  if (!client) {
    return;
  }

  if (socket && client.socket !== socket) {
    return;
  }

  unityClients.delete(clientId);
  broadcastWeb({ type: 'unity_disconnected', clientId, reason });
}

function handleSocketProtocolFailure(socket, reason) {
  if (socket.role === 'unity' && socket.boundClientId) {
    removeUnityClient(socket.boundClientId, reason, socket);
    return;
  }

  if (socket.role === 'web') {
    webClients.delete(socket);
    detachController(socket, reason);
  }
}

function getUnityClientSnapshot() {
  return Array.from(unityClients.values()).map(toPublicClient);
}

function toPublicClient(client) {
  const online = isSocketOpen(client.socket);
  const running = isClientRunning(client.clientId);
  return {
    clientId: client.clientId,
    name: client.name,
    platform: client.platform,
    unityVersion: client.unityVersion,
    methods: client.methods,
    connectedAt: new Date(client.connectedAt).toISOString(),
    lastSeenAt: new Date(client.lastSeenAt).toISOString(),
    availabilityChangedAt: new Date(client.availabilityChangedAt).toISOString(),
    online,
    available: Boolean(client.available),
    unavailableReason: client.unavailableReason || '',
    running,
    lock: toPublicLock(client.lock),
  };
}

function toPublicLock(lock) {
  if (!lock) {
    return null;
  }

  const controller = controllers.get(lock.ownerId);
  return {
    ownerId: lock.ownerId,
    ownerType: lock.ownerType,
    ownerConnected: controller ? controller.sockets.size > 0 : false,
    acquiredAt: new Date(lock.acquiredAt).toISOString(),
    lastSeenAt: new Date(lock.lastSeenAt).toISOString(),
  };
}

function attachController(socket, url) {
  const requestedId = url.searchParams.get('controllerId');
  const controllerId = requestedId && requestedId.trim() ? requestedId.trim() : crypto.randomUUID();
  const controllerType = (url.searchParams.get('controllerType') || url.searchParams.get('controller') || 'web').trim() || 'web';
  const now = Date.now();
  let controller = controllers.get(controllerId);

  if (!controller) {
    controller = {
      id: controllerId,
      type: controllerType,
      sockets: new Set(),
      createdAt: now,
      lastSeenAt: now,
      lastDisconnectedAt: null,
    };
    controllers.set(controllerId, controller);
  }

  controller.type = controllerType;
  controller.lastSeenAt = now;
  controller.sockets.add(socket);
  socket.controller = controller;

  return controller;
}

function detachController(socket, reason) {
  const controller = socket.controller;
  if (!controller) {
    return;
  }

  controller.sockets.delete(socket);
  controller.lastDisconnectedAt = Date.now();
  socket.controller = null;

  if (controller.sockets.size === 0) {
    releaseIdleLocksForController(controller.id, reason);
  }
}

function createTransientController(type) {
  return {
    id: `${type}:${crypto.randomUUID()}`,
    type,
    sockets: new Set(),
    createdAt: Date.now(),
    lastSeenAt: Date.now(),
    lastDisconnectedAt: null,
  };
}

function toPublicController(controller) {
  return {
    ownerId: controller.id,
    ownerType: controller.type,
  };
}

function addHistory(item) {
  if (item.requestId) {
    const existingIndex = executionHistory.findIndex((historyItem) => historyItem.requestId === item.requestId);
    if (existingIndex >= 0) {
      executionHistory.splice(existingIndex, 1, { ...executionHistory[existingIndex], ...item });
      return;
    }
  }

  executionHistory.unshift(item);
  if (executionHistory.length > HISTORY_LIMIT) {
    executionHistory.length = HISTORY_LIMIT;
  }
}

function findHistoryByRequestId(requestId) {
  if (!requestId) {
    return null;
  }
  return executionHistory.find((item) => item.requestId === requestId) || null;
}

function broadcastWeb(message) {
  for (const socket of webClients) {
    send(socket, message);
  }
}

function send(socket, message) {
  if (!isSocketOpen(socket)) {
    return false;
  }

  try {
    socket.send(JSON.stringify(message), (error) => {
      if (error) {
        handleSendFailure(socket, error);
      }
    });
    return true;
  } catch (error) {
    handleSendFailure(socket, error);
    return false;
  }
}

function handleSendFailure(socket, _error) {
  if (socket.role === 'web') {
    webClients.delete(socket);
    detachController(socket, 'send_error');
  }
}

function parseMessage(payload) {
  try {
    return JSON.parse(payload.toString());
  } catch {
    return null;
  }
}

function isSocketOpen(socket) {
  return socket && socket.readyState === WebSocket.OPEN;
}

function toPositiveInteger(value, fallback) {
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed > 0 ? Math.floor(parsed) : fallback;
}

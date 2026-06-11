<template>
  <div v-if="authReady && isAuthenticated" class="app-shell">
    <aside class="nav-sidebar">
      <div class="brand">
        <div>
          <p class="eyebrow">QA Test</p>
          <h1>控制台</h1>
        </div>
        <el-tag :type="wsStatusType" effect="plain">{{ wsStatusText }}</el-tag>
      </div>

      <nav class="nav-menu" aria-label="主导航">
        <RouterLink v-for="item in navItems" :key="item.to" class="nav-link" :to="item.to">
          <el-icon>
            <component :is="item.icon" />
          </el-icon>
          <span>{{ item.label }}</span>
        </RouterLink>
      </nav>

      <div class="nav-summary">
        <div>
          <span>在线实例</span>
          <strong>{{ clients.length }}</strong>
        </div>
        <div>
          <span>执行记录</span>
          <strong>{{ history.length }}</strong>
        </div>
        <div>
          <span>请求序列</span>
          <strong>{{ sequences.length }}</strong>
        </div>
      </div>

      <el-button class="nav-refresh" :icon="Refresh" @click="refreshAll">刷新数据</el-button>
      <el-button v-if="tokenRequired" class="nav-refresh" :icon="SwitchButton" @click="logout">退出登录</el-button>
    </aside>

    <main class="main-panel">
      <RouterView />
    </main>
  </div>

  <div v-else class="login-shell">
    <form v-if="authReady" class="login-panel" @submit.prevent="login">
      <div class="login-heading">
        <p class="eyebrow">QA Test</p>
        <h1>控制台登录</h1>
      </div>

      <el-input
        v-model="loginToken"
        :prefix-icon="Lock"
        autocomplete="current-password"
        placeholder="访问 token"
        show-password
        type="password"
      />
      <p v-if="authError" class="login-error">{{ authError }}</p>
      <el-button class="login-submit" :loading="authLoading" native-type="submit" type="primary">登录</el-button>
    </form>

    <div v-else class="login-panel login-panel-loading">
      <p class="eyebrow">QA Test</p>
      <h1>正在检查登录状态</h1>
    </div>
  </div>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { RouterLink, RouterView } from 'vue-router';
import { Clock, Lock, Monitor, Refresh, SwitchButton, Tickets } from '@element-plus/icons-vue';

import { useQaStore } from './composables/useQaStore';

const { clients, history, refreshAll, sequences, start, stop, wsStatus } = useQaStore();

const authReady = ref(false);
const authLoading = ref(false);
const authError = ref('');
const isAuthenticated = ref(false);
const loginToken = ref('');
const tokenRequired = ref(false);

const navItems = [
  { label: '测试控制台', to: '/', icon: Monitor },
  { label: '请求序列', to: '/sequences', icon: Tickets },
  { label: '执行记录', to: '/history', icon: Clock },
];

const wsStatusText = computed(() => {
  if (wsStatus.value === 'connected') return '已连接';
  if (wsStatus.value === 'connecting') return '连接中';
  return '已断开';
});
const wsStatusType = computed(() => (wsStatus.value === 'connected' ? 'success' : wsStatus.value === 'connecting' ? 'warning' : 'danger'));

onMounted(() => {
  checkAuth();
});

onBeforeUnmount(() => {
  stop();
});

async function checkAuth() {
  authLoading.value = true;
  authError.value = '';

  try {
    const response = await fetch('/api/web-auth');
    const payload = await readJson(response);
    if (!response.ok) {
      throw new Error(payload.error || `登录状态检查失败：${response.status}`);
    }

    tokenRequired.value = Boolean(payload.tokenRequired);
    isAuthenticated.value = Boolean(payload.authenticated);
    if (isAuthenticated.value) {
      start();
    }
  } catch (error) {
    authError.value = error.message || '登录状态检查失败';
  } finally {
    authReady.value = true;
    authLoading.value = false;
  }
}

async function login() {
  if (!loginToken.value.trim()) {
    authError.value = '请输入访问 token';
    return;
  }

  authLoading.value = true;
  authError.value = '';

  try {
    const response = await fetch('/api/web-login', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
      },
      body: JSON.stringify({ token: loginToken.value }),
    });
    const payload = await readJson(response);
    if (!response.ok) {
      throw new Error(payload.error || '登录失败');
    }

    tokenRequired.value = Boolean(payload.tokenRequired);
    isAuthenticated.value = Boolean(payload.authenticated);
    loginToken.value = '';
    if (isAuthenticated.value) {
      start();
    }
  } catch (error) {
    authError.value = error.message || '登录失败';
  } finally {
    authLoading.value = false;
  }
}

async function logout() {
  authLoading.value = true;
  stop();

  try {
    await fetch('/api/web-logout', { method: 'POST' });
  } finally {
    isAuthenticated.value = false;
    loginToken.value = '';
    authLoading.value = false;
  }
}

async function readJson(response) {
  const text = await response.text();
  return text ? JSON.parse(text) : {};
}
</script>

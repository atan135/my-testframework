import { createRouter, createWebHistory } from 'vue-router';

export const router = createRouter({
  history: createWebHistory(),
  routes: [
    {
      path: '/',
      name: 'console',
      component: () => import('../views/ClientConsoleView.vue'),
    },
    {
      path: '/history',
      name: 'history',
      component: () => import('../views/ExecutionHistoryView.vue'),
    },
    {
      path: '/sequences',
      name: 'sequences',
      component: () => import('../views/RequestSequencesView.vue'),
    },
    {
      path: '/:pathMatch(.*)*',
      redirect: '/',
    },
  ],
});

import { createApp } from 'vue';
import { ElButton } from 'element-plus/es/components/button/index.mjs';
import { ElDialog } from 'element-plus/es/components/dialog/index.mjs';
import { ElEmpty } from 'element-plus/es/components/empty/index.mjs';
import { ElForm, ElFormItem } from 'element-plus/es/components/form/index.mjs';
import { ElIcon } from 'element-plus/es/components/icon/index.mjs';
import { ElInput } from 'element-plus/es/components/input/index.mjs';
import { ElInputNumber } from 'element-plus/es/components/input-number/index.mjs';
import { ElScrollbar } from 'element-plus/es/components/scrollbar/index.mjs';
import { ElOption, ElSelect } from 'element-plus/es/components/select/index.mjs';
import { ElSwitch } from 'element-plus/es/components/switch/index.mjs';
import { ElTable, ElTableColumn } from 'element-plus/es/components/table/index.mjs';
import { ElTag } from 'element-plus/es/components/tag/index.mjs';
import 'element-plus/dist/index.css';

import App from './App.vue';
import { router } from './router';
import './styles.css';

const elementPlusComponents = [
  ElButton,
  ElDialog,
  ElEmpty,
  ElForm,
  ElFormItem,
  ElIcon,
  ElInput,
  ElInputNumber,
  ElOption,
  ElScrollbar,
  ElSelect,
  ElSwitch,
  ElTable,
  ElTableColumn,
  ElTag,
];

const app = createApp(App);

for (const component of elementPlusComponents) {
  app.component(component.name, component);
}

app.use(router).mount('#app');

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue';
import {
  call,
  connectionView,
  controls,
  failure,
  isHttpOrigin,
  labels,
  type Command,
  type Failure,
  type LogEntry,
  type Status,
} from './services/client';

// ── 响应式状态 ──────────────────────────────────────────────
const server = ref('https://');
const accessCode = ref('');
const password = ref('');
const allowHttp = ref(false);

function onAccessCodeBlur() {
  accessCode.value = accessCode.value.trim().toUpperCase();
}

const status = ref<Status | null>(null);
const logs = ref<LogEntry[]>([]);
const copyStatus = ref<'idle' | 'success' | 'error'>('idle');
const error = ref<Failure | null>(null);

async function writeClipboard(text: string): Promise<boolean> {
  // 方法1：execCommand 兼容方案（在 WebView2 / 本地服务开发环境下最高效可靠）
  try {
    const textarea = document.createElement('textarea');
    textarea.value = text;
    textarea.style.position = 'fixed';
    textarea.style.left = '0';
    textarea.style.top = '0';
    textarea.style.width = '1px';
    textarea.style.height = '1px';
    textarea.style.padding = '0';
    textarea.style.border = 'none';
    textarea.style.outline = 'none';
    textarea.style.background = 'transparent';
    document.body.appendChild(textarea);
    textarea.focus();
    textarea.select();
    textarea.setSelectionRange(0, textarea.value.length);
    const ok = document.execCommand('copy');
    document.body.removeChild(textarea);
    if (ok) return true;
  } catch (e) {
    console.warn('execCommand copy failed:', e);
  }

  // 方法2：navigator.clipboard API
  if (navigator.clipboard && typeof navigator.clipboard.writeText === 'function') {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch (e) {
      console.warn('navigator.clipboard.writeText failed:', e);
    }
  }

  return false;
}

async function copyLogs() {
  if (!logs.value.length) return;
  const text = logs.value
    .map((l) => `[${new Date(l.time * 1000).toLocaleTimeString()}] ${l.message}`)
    .join('\n');

  const success = await writeClipboard(text);
  copyStatus.value = success ? 'success' : 'error';
  setTimeout(() => {
    copyStatus.value = 'idle';
  }, 2500);
}

function handleLogRightClick(e: MouseEvent) {
  const selection = window.getSelection()?.toString().trim();
  if (selection) {
    e.preventDefault();
    void writeClipboard(selection).then((success) => {
      copyStatus.value = success ? 'success' : 'error';
      setTimeout(() => {
        copyStatus.value = 'idle';
      }, 2500);
    });
  }
}
const busy = ref(false);
const available = ref(false);
const refreshing = ref(false);

// ── 派生计算 ────────────────────────────────────────────────
const buttons = computed(() => controls(available.value ? status.value : null, busy.value));
const identity = computed(() => status.value?.configured ?? status.value?.session);
const shownError = computed(() => error.value ?? (available.value ? status.value?.last_error : null));
const httpServer = computed(() => isHttpOrigin(server.value));
const tunnel = computed(() => connectionView(available.value ? status.value : null));

// ── 定时刷新 ────────────────────────────────────────────────
let timer: ReturnType<typeof setInterval> | undefined;

async function refresh(manual = false) {
  if (refreshing.value) return;
  refreshing.value = true;
  try {
    status.value = await call<Status>('get_status');
    available.value = true;
    if (manual) error.value = null;
    logs.value = await call<LogEntry[]>('get_recent_logs');
  } catch (e) {
    available.value = false;
    error.value = failure(e);
  } finally {
    refreshing.value = false;
  }
}

// ── 用户操作 ────────────────────────────────────────────────
async function act(command: Command) {
  if (busy.value) return;
  busy.value = true;
  error.value = null;
  try {
    const normalizedCode = accessCode.value.trim().toUpperCase();
    accessCode.value = normalizedCode;
    const args =
      command === 'login'
        ? {
            server: server.value.trim(),
            accessCode: normalizedCode,
            password: password.value,
            allowHttp: allowHttp.value,
          }
        : undefined;
    status.value = await call<Status>(command, args);
    available.value = true;
  } catch (e) {
    error.value = failure(e);
  } finally {
    if (command === 'login') password.value = '';
    busy.value = false;
    await refresh();
  }
}

onMounted(() => {
  void refresh();
  timer = setInterval(() => void refresh(), 2000);
});

onUnmounted(() => clearInterval(timer));
</script>

<template>
  <div class="shell">
    <!-- 侧边栏 -->
    <aside>
      <div class="brand">
        <span class="brand-mark">t</span>
        <div>
          Tinc <strong>Connect</strong>
          <small>安全连接你的网络</small>
        </div>
      </div>
      <div class="nav-item">
        <span>◉</span> 我的连接
      </div>
      <div class="aside-note">
        <span class="small-dot"></span> 本地服务托管 VPN
        <p>关闭窗口后，已启动的 VPN 可继续运行。</p>
      </div>
      <footer>WINDOWS / LINUX <span>0.1.0</span></footer>
    </aside>

    <!-- 主内容区 -->
    <main>
      <!-- 顶部标题 -->
      <header>
        <div>
          <span class="eyebrow">YOUR PRIVATE NETWORK</span>
          <h1>连接，从这里开始</h1>
          <p>登录节点，应用配置，再启动你的专属网络。</p>
        </div>
        <button class="subtle" :disabled="refreshing" @click="refresh(true)">↻ 刷新状态</button>
      </header>

      <!-- 错误提示 -->
      <div v-if="shownError" class="error" role="alert">
        <strong>{{ shownError.message }}</strong>
        <small>
          错误代码：{{ shownError.code }}
          {{ shownError.retryable ? '· 可以重试' : '' }}
        </small>
      </div>

      <!-- VPN 状态卡片 -->
      <div class="status-card">
        <div class="status-orb" :class="{ running: available && status?.process_running }">◎</div>
        <div class="status-copy">
          <span class="eyebrow">VPN STATUS</span>
          <h2>{{ available && status ? labels[status.phase] : '本地服务未连接' }}</h2>
          <p>{{ available ? status?.stage : '请安装或启动 Tinc Client Agent，随后刷新状态。' }}</p>
        </div>
        <div class="connection">
          <span>隧道连接</span>
          <strong>{{ tunnel.label }}</strong>
          <small>{{ tunnel.detail }}</small>
        </div>
      </div>

      <!-- 操作区：登录 + 配置连接 -->
      <div class="columns">
        <!-- 01 登录节点 -->
        <section class="card">
          <div class="section-title">
            <span class="step">01</span>
            <div>
              <h2>登录节点</h2>
              <p>接入码由管理员创建节点后提供</p>
            </div>
          </div>

          <form @submit.prevent="act('login')">
            <label>
              TincLink 服务地址
              <input
                v-model="server"
                placeholder="https://vpn.example.com"
                autocomplete="url"
                :disabled="busy"
              />
            </label>
            <label>
              接入码
              <input
                v-model="accessCode"
                placeholder="TL-7K9M-2Q4F"
                autocomplete="username"
                maxlength="16"
                :disabled="busy"
                @blur="onAccessCodeBlur"
              />
            </label>
            <label>
              密码
              <input
                v-model="password"
                type="password"
                placeholder="请输入节点密码"
                autocomplete="current-password"
                :disabled="busy"
              />
            </label>
            <label class="checkbox">
              <input v-model="allowHttp" type="checkbox" :disabled="busy || !httpServer" />
              允许 HTTP 明文登录（仅开发环境）
            </label>
            <p v-if="httpServer && !allowHttp" class="http-warning">
              当前地址使用 HTTP，必须勾选上方选项后才能登录。
            </p>
            <button
              class="primary full"
              :disabled="busy || !available || !accessCode.trim() || !password || (httpServer && !allowHttp)"
            >
              {{ busy ? '操作处理中…' : status?.session ? '重新登录' : '登录节点' }}
              <span>→</span>
            </button>
          </form>

          <div v-if="status?.session" class="session">
            已登录节点：{{ status.session.sid }}
            <span v-if="status.session.access_code" class="session-code">（{{ status.session.access_code }}）</span>
            <button class="text-button" :disabled="busy || !available" @click="act('logout')">
              退出登录
            </button>
          </div>
        </section>

        <!-- 02 配置与连接 -->
        <section class="card">
          <div class="section-title">
            <span class="step">02</span>
            <div>
              <h2>配置与连接</h2>
              <p>密钥生成、交换和配置校验由服务完成</p>
            </div>
          </div>

          <dl>
            <div>
              <dt>节点名称</dt>
              <dd>{{ identity?.sid ?? '等待登录' }}</dd>
            </div>
            <div>
              <dt>所属网络</dt>
              <dd>{{ identity?.net_name ?? '等待登录' }}</dd>
            </div>
            <div>
              <dt>节点地址</dt>
              <dd>{{ identity?.node_ip ?? '由后台分配' }}</dd>
            </div>
            <div v-if="identity?.access_code">
              <dt>接入码</dt>
              <dd>{{ identity.access_code }}</dd>
            </div>
            <div>
              <dt>服务端状态</dt>
              <dd>
                {{ status?.confirmation === 'ready' ? '网络已就绪' : '等待配置' }}
              </dd>
            </div>
          </dl>

          <div class="hint">
            首次使用需由管理员在后台创建网络并关联客户节点。配置失败后可重试，本机密钥会保留。
          </div>

          <button class="secondary full" :disabled="!buttons.configure" @click="act('configure')">
            {{ status?.configured ? '更新配置（高级）' : '下载并应用配置' }}
          </button>

          <button class="text-button full" :disabled="!buttons.rotate" @click="act('rotate_key')">
            安全轮换当前节点密钥
          </button>

          <div class="actions">
            <button class="primary" :disabled="!buttons.start" @click="act('start_vpn')">
              ▶ 启动 VPN
            </button>
            <button class="secondary" :disabled="!buttons.stop" @click="act('stop_vpn')">
              ■ 停止
            </button>
          </div>

          <p class="fine">
            配置成功表示服务端网络已就绪。启动后 Agent 将自动设置精确路由并验证连通性。
          </p>
        </section>
      </div>

      <!-- 活动日志 -->
      <section class="card logs">
        <div class="log-heading">
          <h2>活动日志</h2>
          <div class="log-actions">
            <span>本地服务 · 最近 {{ logs.length }} 条</span>
            <button
              class="copy-button"
              :class="{ error: copyStatus === 'error' }"
              :disabled="!logs.length"
              @click="copyLogs"
            >
              {{
                copyStatus === 'success'
                  ? '✓ 已复制'
                  : copyStatus === 'error'
                  ? '❌ 复制失败'
                  : '📋 复制日志'
              }}
            </button>
          </div>
        </div>
        <div class="log-body" aria-live="polite" @contextmenu="handleLogRightClick">
          <p v-if="!logs.length" class="empty">
            暂无活动。服务连接后将在这里显示实际操作结果。
          </p>
          <div
            v-for="(line, i) in logs"
            :key="`${line.time}-${i}`"
            class="log-line"
          >
            <time>{{ new Date(line.time * 1000).toLocaleTimeString() }}</time>
            <span>{{ line.message }}</span>
          </div>
        </div>
      </section>
    </main>
  </div>
</template>

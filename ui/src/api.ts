export interface Account { id: string; label: string; username: string; service: string; enabled: boolean; note: string; last_result: string; last_ok: boolean | null; has_password: boolean }
export interface AccountInput { id?: string; label: string; username: string; password: string; service: string; enabled: boolean; note: string }
export interface Reconnect { enabled: boolean; interval_seconds: number; fail_threshold: number }
export interface Connection { profile_id: string; display_name: string; gateway: string; service: string; user_group: string; probe_urls: string[]; discovery_urls: string[]; timeout_seconds: number; retry_per_account: number; retry_interval_seconds: number; password_encrypt: boolean }
export interface Snapshot { accounts: Account[]; reconnect: Reconnect; logs: string[]; config: Connection }
export interface Status { online: boolean | null; network_online: boolean | null; portal_online: boolean | null; reason: string; message: string; username: string; service: string; local_ip: string; checked_at: number }
export interface Power { enabled: boolean; shutdown_time: string; wake_time: string; days: string[]; shutdown_delay_seconds: number; wake_method: string; shutdown_action: string }
export interface Profile { id: string; name: string; description: string; config: Connection }
export interface Service { name: string; value: string }
export interface ActionResult { ok: boolean; message?: string; error?: string; permission_error?: boolean; admin_command?: string; pending_config?: string; [key: string]: unknown }
declare global { interface Window { __TAURI_INTERNALS__?: unknown } }
export const isDesktop = !!window.__TAURI_INTERNALS__;
export interface AppInfo { version: string; data_dir: string; platform: string; mode?: string; desktop?: boolean }
export interface Session { authenticated: boolean; setup_required: boolean; username: string }
export interface Countdown { ok: boolean; active: boolean; action?: string; execute_at?: number; remaining_seconds?: number }
export interface UpdateInfo { ok: boolean; available: boolean; current_version: string; latest_version?: string; release_url?: string; notes?: string; message?: string; assets?: {name:string;url:string;size:number}[] }
export interface Diagnostics { ok:boolean; filename:string; content:string; mime_type:string }
export class ApiError extends Error { constructor(message:string,public status?:number) { super(message); } }
export const defaultConnection: Connection = { profile_id: 'qhu', display_name: '青海大学', gateway: 'http://210.27.177.172', service: '校园移动', user_group: '本科生组', probe_urls: ['http://connect.rom.miui.com/generate_204', 'http://www.baidu.com'], discovery_urls: ['http://www.msftconnecttest.com/redirect'], timeout_seconds: 8, retry_per_account: 2, retry_interval_seconds: 3, password_encrypt: false };
export const defaultPower: Power = { enabled: false, shutdown_time: '23:00', wake_time: '07:00', days: ['mon','tue','wed','thu','fri','sat','sun'], shutdown_delay_seconds: 60, wake_method: 'timer', shutdown_action: 'suspend' };
export async function api<T>(method: string, payload?: unknown): Promise<T> {
  let value: unknown;
  if (isDesktop) {
    const {invoke} = await import('@tauri-apps/api/core');
    value = await invoke('dispatch', {method, params: payload ?? null});
  }
  else {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), method === 'connect' ? 135000 : 30000);
    try {
      const response = await fetch('/api/' + method, { method: payload === undefined ? 'GET' : 'POST', headers: payload === undefined ? {} : {'Content-Type': 'application/json'}, credentials:'same-origin', body: payload === undefined ? undefined : JSON.stringify(payload), signal: controller.signal });
      value = await response.json();
      if (!response.ok) throw new ApiError(response.status === 401 ? '登录已过期，请重新登录。' : (value as {error?:string}).error || `请求失败（${response.status}）`,response.status);
    } finally { clearTimeout(timeout); }
  }
  if (value && typeof value === 'object' && 'error' in value && !('ok' in value)) throw new Error(String((value as {error:unknown}).error));
  return value as T;
}

// Reserved tag for stashing accounts (hidden by default)
export const STASH_TAG = "stashed";

export interface MachineIds {
  "telemetry.devDeviceId": string;
  "telemetry.macMachineId": string;
  "telemetry.machineId": string;
  "telemetry.sqmId": string;
  "storage.serviceMachineId": string;
  "system.machineGuid"?: string;
  "system.sqmClientId"?: string;
}

export interface AccountInfo {
  email: string;
  token: string;
  refresh_token?: string;
  workos_cursor_session_token?: string;
  stripe_url?: string;
  is_current: boolean;
  created_at: string;
  username?: string;
  tags?: string[];
  machine_ids?: MachineIds;
  // 订阅信息
  subscription_type?: string;
  subscription_status?: string;
  trial_days_remaining?: number;
  // 用量费用（单位：cents，来自缓存的用量数据）
  usage_cost_cents?: number;
  // /api/auth/me 返回的用户信息
  name?: string;
  sub?: string;
  picture?: string;
  user_id?: number;
}

/** /api/auth/me 接口返回 */
export interface AuthMeResponse {
  email: string;
  email_verified: boolean;
  name: string;
  sub: string;
  created_at: string;
  updated_at: string;
  picture: string | null;
  id: number;
}

export interface AccountListResult {
  success: boolean;
  accounts: AccountInfo[];
  current_account: AccountInfo | null;
  message: string;
}

export interface SwitchAccountResult {
  success: boolean;
  message: string;
  details: string[];
}

export interface AddAccountResult {
  success: boolean;
  message: string;
}

export interface EditAccountResult {
  success: boolean;
  message: string;
}

export interface BatchUpdateTagsResult {
  success: boolean;
  successCount: number;
  failCount: number;
  skippedCount: number;
  message: string;
}

export interface RemoveAccountResult {
  success: boolean;
  message: string;
}

export interface LogoutResult {
  success: boolean;
  message: string;
  details: string[];
}

export interface ExportAccountsResult {
  success: boolean;
  message: string;
  exported_path: string;
}

export interface ImportAccountsResult {
  success: boolean;
  message: string;
}

export interface DeleteAccountResult {
  success: boolean;
  message: string;
}

/** 无感换号状态 */
export interface SeamlessStatus {
  injected: boolean;
  server_running: boolean;
  port: number;
  backup_exists: boolean;
}

/** 无感换号操作结果 */
export interface SeamlessResult {
  success: boolean;
  message: string;
  details?: string[];
  port?: number;
}



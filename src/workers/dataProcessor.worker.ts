/**
 * Web Worker for Heavy Data Processing
 * 处理大量数据，避免阻塞主线程
 * 
 * ✅ 功能:
 * 1. 账号数据解析和验证
 * 2. 大文件导入处理
 * 3. 数据聚合和统计
 * 4. 数据导出格式化
 * 
 * 使用场景:
 * - 导入 10000+ 账号
 * - 导出大量数据
 * - 复杂的数据处理
 */

// Worker 消息类型
export type WorkerMessageType =
  | 'PARSE_ACCOUNTS'
  | 'VALIDATE_ACCOUNTS'
  | 'AGGREGATE_USAGE'
  | 'EXPORT_DATA'
  | 'DEDUPLICATE_ACCOUNTS';

// Worker 消息接口
export interface WorkerMessage {
  type: WorkerMessageType;
  id: string; // 请求 ID
  data: any;
}

// Worker 响应接口
export interface WorkerResponse {
  type: WorkerMessageType;
  id: string;
  success: boolean;
  data?: any;
  error?: string;
  progress?: number; // 进度 0-100
}

// 账号接口（简化版）
interface AccountInfo {
  email: string;
  token: string;
  refresh_token?: string | null;
  workos_cursor_session_token?: string | null;
  is_current?: boolean;
  created_at?: string;
  username?: string | null;
}

/**
 * 解析账号数据
 */
function parseAccounts(jsonString: string): {
  success: boolean;
  accounts?: AccountInfo[];
  error?: string;
} {
  try {
    let parsed = JSON.parse(jsonString);
    
    // Support single object (wrap in array)
    if (!Array.isArray(parsed)) {
      if (typeof parsed === 'object' && parsed !== null) {
        parsed = [parsed];
      } else {
        return {
          success: false,
          error: '数据格式错误：应为数组或对象',
        };
      }
    }

    const accounts: AccountInfo[] = parsed.map((item, index) => {
      // Support alternative field names: accessToken -> token
      const token = item.token || item.accessToken;
      const email = item.email;
      
      if (!email || !token) {
        throw new Error(`第 ${index + 1} 个账号缺少必需字段 (email 或 token/accessToken)`);
      }

      return {
        email: email,
        token: token,
        refresh_token: item.refresh_token || item.refreshToken || null,
        workos_cursor_session_token: item.workos_cursor_session_token || item.workosCursorSessionToken || null,
        is_current: item.is_current || item.isCurrent || false,
        created_at: item.created_at || item.createdAt || new Date().toISOString(),
        username: item.username || null,
      };
    });

    return {
      success: true,
      accounts,
    };
  } catch (error: any) {
    return {
      success: false,
      error: error.message || '解析失败',
    };
  }
}

/**
 * 验证账号数据
 */
function validateAccounts(accounts: AccountInfo[]): {
  success: boolean;
  validAccounts?: AccountInfo[];
  invalidCount?: number;
  errors?: string[];
} {
  const validAccounts: AccountInfo[] = [];
  const errors: string[] = [];

  accounts.forEach((account, index) => {
    // 验证邮箱格式
    const emailRegex = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
    if (!emailRegex.test(account.email)) {
      errors.push(`第 ${index + 1} 个账号邮箱格式无效: ${account.email}`);
      return;
    }

    // 验证 token 不为空
    if (!account.token || account.token.trim() === '') {
      errors.push(`第 ${index + 1} 个账号 token 为空`);
      return;
    }

    validAccounts.push(account);
  });

  return {
    success: true,
    validAccounts,
    invalidCount: errors.length,
    errors: errors.slice(0, 10), // 只返回前 10 个错误
  };
}

/**
 * 去重账号（基于邮箱）
 */
function deduplicateAccounts(accounts: AccountInfo[]): {
  success: boolean;
  uniqueAccounts?: AccountInfo[];
  duplicateCount?: number;
} {
  const seen = new Set<string>();
  const uniqueAccounts: AccountInfo[] = [];
  let duplicateCount = 0;

  accounts.forEach((account) => {
    if (!seen.has(account.email)) {
      seen.add(account.email);
      uniqueAccounts.push(account);
    } else {
      duplicateCount++;
    }
  });

  return {
    success: true,
    uniqueAccounts,
    duplicateCount,
  };
}

/**
 * 聚合用量数据
 */
function aggregateUsage(usageData: any[]): {
  success: boolean;
  aggregated?: any;
} {
  try {
    const aggregated = {
      totalRequests: 0,
      totalTokens: 0,
      totalAccounts: usageData.length,
      byModel: {} as Record<string, number>,
      byDate: {} as Record<string, number>,
    };

    usageData.forEach((data) => {
      if (data.requests) {
        aggregated.totalRequests += data.requests;
      }
      if (data.tokens) {
        aggregated.totalTokens += data.tokens;
      }
      if (data.model) {
        aggregated.byModel[data.model] = (aggregated.byModel[data.model] || 0) + 1;
      }
      if (data.date) {
        aggregated.byDate[data.date] = (aggregated.byDate[data.date] || 0) + 1;
      }
    });

    return {
      success: true,
      aggregated,
    };
  } catch (error: any) {
    return {
      success: false,
    };
  }
}

/**
 * 导出数据为 JSON 字符串
 */
function exportData(data: any, options: { pretty?: boolean } = {}): {
  success: boolean;
  jsonString?: string;
  error?: string;
} {
  try {
    const jsonString = options.pretty
      ? JSON.stringify(data, null, 2)
      : JSON.stringify(data);

    return {
      success: true,
      jsonString,
    };
  } catch (error: any) {
    return {
      success: false,
      error: error.message || '导出失败',
    };
  }
}

/**
 * Worker 消息处理
 */
self.onmessage = (event: MessageEvent<WorkerMessage>) => {
  const { type, id, data } = event.data;

  let response: WorkerResponse;

  try {
    switch (type) {
      case 'PARSE_ACCOUNTS': {
        const result = parseAccounts(data.jsonString);
        response = {
          type,
          id,
          success: result.success,
          data: result.accounts,
          error: result.error,
        };
        break;
      }

      case 'VALIDATE_ACCOUNTS': {
        const result = validateAccounts(data.accounts);
        response = {
          type,
          id,
          success: result.success,
          data: {
            validAccounts: result.validAccounts,
            invalidCount: result.invalidCount,
            errors: result.errors,
          },
        };
        break;
      }

      case 'DEDUPLICATE_ACCOUNTS': {
        const result = deduplicateAccounts(data.accounts);
        response = {
          type,
          id,
          success: result.success,
          data: {
            uniqueAccounts: result.uniqueAccounts,
            duplicateCount: result.duplicateCount,
          },
        };
        break;
      }

      case 'AGGREGATE_USAGE': {
        const result = aggregateUsage(data.usageData);
        response = {
          type,
          id,
          success: result.success,
          data: result.aggregated,
        };
        break;
      }

      case 'EXPORT_DATA': {
        const result = exportData(data.data, data.options);
        response = {
          type,
          id,
          success: result.success,
          data: result.jsonString,
          error: result.error,
        };
        break;
      }

      default:
        response = {
          type,
          id,
          success: false,
          error: `未知的消息类型: ${type}`,
        };
    }
  } catch (error: any) {
    response = {
      type,
      id,
      success: false,
      error: error.message || '处理失败',
    };
  }

  // 发送响应
  self.postMessage(response);
};

// 导出类型（用于主线程）
export {};


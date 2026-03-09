import { useState, useCallback, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AccountService } from "../services/accountService";
import { ConfigService } from "../services/configService";
import { AccountUsageService } from "../services/accountUsageService";
import type { AccountListResult, AccountInfo } from "../types/account";
import { STASH_TAG } from "../types/account";
import { performanceMonitor } from "../utils/performance";
import { safeStorage } from "../utils/safeStorage";

export const useAccountManagement = () => {
  const [accountData, setAccountData] = useState<AccountListResult | null>(null);
  const [loading, setLoading] = useState(true);
  const [selectedAccounts, setSelectedAccounts] = useState<Set<string>>(new Set());
  const [subscriptionFilter, setSubscriptionFilter] = useState<string>("all");
  // Default to showing unstashed accounts only
  const [tagFilter, setTagFilter] = useState<string>("__unstashed__");
  const [sortField, setSortField] = useState<string>(() => {
    return safeStorage.get<string>('account_sort_field', 'usage', true) || 'usage';
  });
  const [sortOrder, setSortOrder] = useState<'asc' | 'desc'>(() => {
    return safeStorage.get<'asc' | 'desc'>('account_sort_order', 'desc', true) || 'desc';
  });
  const [refreshProgress, setRefreshProgress] = useState<{
    current: number;
    total: number;
    isRefreshing: boolean;
  }>({ current: 0, total: 0, isRefreshing: false });
  const [concurrentLimit, setConcurrentLimit] = useState<number>(() => {
    // ✅ 使用安全包装器读取配置
    const limit = safeStorage.get<number>('refresh_concurrent_limit', 5, true);
    return limit !== null && limit >= 1 && limit <= 10 ? limit : 5;
  });

  // 从数据库读取当前账号，并更新 state 中的 current_account 和 is_current 标记
  const refreshCurrentAccount = useCallback(async () => {
    try {
      const currentAccount = await AccountService.getCurrentAccount();
      setAccountData((prev) => {
        if (!prev) return prev;
        const accounts = prev.accounts.map((acc) => ({
          ...acc,
          is_current: currentAccount ? acc.email === currentAccount.email : false,
        }));
        return { ...prev, accounts, current_account: currentAccount };
      });
      return { success: true, currentAccount };
    } catch (error) {
      console.error("Failed to refresh current account:", error);
      return { success: false, currentAccount: null };
    }
  }, []);

  // 加载账户的用量费用（从缓存）
  const loadUsageCostsForAccounts = useCallback(async (accounts: AccountInfo[]): Promise<Map<string, number>> => {
    const costMap = new Map<string, number>();
    
    // 并行加载所有账户的用量缓存
    const promises = accounts.map(async (account) => {
      try {
        const cacheResult = await AccountUsageService.loadAccountUsageCache(account.email);
        if (cacheResult.success && cacheResult.data?.aggregatedData) {
          const totalCostCents = cacheResult.data.aggregatedData.total_cost_cents || 0;
          costMap.set(account.email, totalCostCents);
        }
      } catch (error) {
        console.warn(`Failed to load usage cache for ${account.email}:`, error);
      }
    });

    await Promise.allSettled(promises);
    return costMap;
  }, []);

  // 加载账户列表
  const loadAccounts = useCallback(async () => {
    performanceMonitor.start('loadAccounts');
    
    try {
      // 1. 先尝试从本地加载缓存数据（立即显示）
      performanceMonitor.start('loadAccountCache');
      const { ConfigService } = await import("../services/configService");
      const cacheResult = await ConfigService.loadAccountCache();
      performanceMonitor.end('loadAccountCache');

      // 如果有缓存，立即显示（同时从 DB 读取当前账号）
      if (cacheResult.success && cacheResult.data && cacheResult.data.length > 0) {
        console.log(`📦 从缓存加载了 ${cacheResult.data.length} 个账户`);
        
        const currentAccount = await AccountService.getCurrentAccount();
        const cachedAccounts = cacheResult.data.map((acc) => ({
          ...acc,
          is_current: currentAccount ? acc.email === currentAccount.email : false,
        }));

        const cachedAccountData: AccountListResult = {
          success: true,
          message: "从缓存加载",
          accounts: cachedAccounts,
          current_account: currentAccount,
        };
        
        setAccountData(cachedAccountData);
        setLoading(false);

        // 后台加载用量费用
        loadUsageCostsForAccounts(cachedAccounts).then((costMap) => {
          if (costMap.size > 0) {
            setAccountData((prev) => {
              if (!prev?.accounts) return prev;
              const accountsWithCost = prev.accounts.map((acc) => ({
                ...acc,
                usage_cost_cents: costMap.get(acc.email) ?? acc.usage_cost_cents,
              }));
              return { ...prev, accounts: accountsWithCost };
            });
          }
        });
      } else {
        setLoading(true);
      }

      // 2. 后台从 API 获取最新数据
      const result = await AccountService.getAccountList();
      
      if (!result.success) {
        if (cacheResult.success && cacheResult.data) {
          console.log("⚠️ API 加载失败，但已有缓存数据");
          return { success: true, fromCache: true };
        }
        return { success: false, message: "加载账户列表失败" };
      }

      // 3. 合并 API 数据和缓存数据
      let finalAccounts = result.accounts;
      let hasIncompleteCache = false;

      if (cacheResult.success && cacheResult.data && cacheResult.data.length > 0) {
        finalAccounts = result.accounts.map((account) => {
          const cached = cacheResult.data?.find((c: any) => c.email === account.email);
          if (cached && cached.subscription_type !== undefined) {
            return {
              ...account,
              subscription_type: cached.subscription_type,
              subscription_status: cached.subscription_status,
              trial_days_remaining: cached.trial_days_remaining,
            };
          } else {
            hasIncompleteCache = true;
            return account;
          }
        });
      } else {
        hasIncompleteCache = result.accounts.length > 0;
      }

      // 4. 加载用量费用（后台执行，不阻塞主流程）
      loadUsageCostsForAccounts(finalAccounts).then((costMap) => {
        if (costMap.size > 0) {
          setAccountData((prev) => {
            if (!prev?.accounts) return prev;
            const accountsWithCost = prev.accounts.map((acc) => ({
              ...acc,
              usage_cost_cents: costMap.get(acc.email) ?? acc.usage_cost_cents,
            }));
            return { ...prev, accounts: accountsWithCost };
          });
        }
      });

      // 5. 更新为最新数据
      setAccountData({
        ...result,
        accounts: finalAccounts,
      });

      return { success: true, hasIncompleteCache };
    } catch (error) {
      console.error("Failed to load accounts:", error);
      return { success: false, message: "加载账户列表失败" };
    } finally {
      setLoading(false);
      const duration = performanceMonitor.end('loadAccounts');
      console.log(`✅ 账户列表加载完成，耗时: ${duration.toFixed(2)}ms`);
    }
  }, []);

  // 刷新单个账户（获取订阅信息 + auth/me 用户信息）
  // Note: index parameter is kept for backward compatibility but not used (we use email lookup instead)
  const refreshSingleAccount = useCallback(async (account: AccountInfo, _index: number) => {
    performanceMonitor.start(`refreshAccount-${account.email}`);

    try {
      setRefreshProgress({ current: 0, total: 1, isRefreshing: true });

      const { ConfigService } = await import("../services/configService");
      const authResult = await ConfigService.refreshSingleAccountInfo(account.token);

      if (authResult.success && authResult.user_info?.account_info) {
        // 如果有 session token，顺便调 auth/me 获取用户详细信息
        let authMeData: Record<string, unknown> = {};
        if (account.workos_cursor_session_token) {
          try {
            const meResult = await AccountService.getAuthMe(account.workos_cursor_session_token);
            if (meResult.success && meResult.data) {
              authMeData = {
                name: meResult.data.name || undefined,
                sub: meResult.data.sub || undefined,
                picture: meResult.data.picture || undefined,
                user_id: meResult.data.id || undefined,
              };
            }
          } catch { /* 静默失败 */ }
        }

        setAccountData((prevData) => {
          if (!prevData?.accounts) return prevData;
          // Fix: Use email lookup instead of index to avoid wrong account update when filtered
          const idx = prevData.accounts.findIndex(a => a.email === account.email);
          if (idx === -1) return prevData;
          
          const updatedAccounts = [...prevData.accounts];
          updatedAccounts[idx] = {
            ...updatedAccounts[idx],
            subscription_type: authResult.user_info.account_info.subscription_type,
            subscription_status: authResult.user_info.account_info.subscription_status,
            trial_days_remaining: authResult.user_info.account_info.trial_days_remaining,
            ...authMeData,
          };

          ConfigService.saveAccountCache(updatedAccounts);

          return { ...prevData, accounts: updatedAccounts };
        });

        setRefreshProgress({ current: 1, total: 1, isRefreshing: true });
        return { success: true };
      }

      // 区分失败原因，token 失效时标记到账号数据
      const status = authResult.user_info?.api_status;
      const errMsg = authResult.user_info?.error_message;
      if (status === 401 || status === 403) {
        const { ConfigService } = await import("../services/configService");
        setAccountData((prevData) => {
          if (!prevData?.accounts) return prevData;
          // Fix: Use email lookup instead of index
          const idx = prevData.accounts.findIndex(a => a.email === account.email);
          if (idx === -1) return prevData;
          
          const updatedAccounts = [...prevData.accounts];
          updatedAccounts[idx] = { ...updatedAccounts[idx], subscription_type: "token_expired" };
          ConfigService.saveAccountCache(updatedAccounts);
          return { ...prevData, accounts: updatedAccounts };
        });
        return { success: false, message: `Token 已失效 (${status})` };
      } else if (errMsg) {
        return { success: false, message: `网络错误: ${errMsg}` };
      }
      return { success: false, message: "刷新失败: 未获取到订阅信息" };
    } catch (error) {
      console.error("刷新账户信息失败:", error);
      return { success: false, message: `请求异常: ${error}` };
    } finally {
      const duration = performanceMonitor.end(`refreshAccount-${account.email}`);
      console.log(`✅ 账户刷新完成: ${account.email}，耗时: ${duration.toFixed(2)}ms`);
      
      setTimeout(() => {
        setRefreshProgress({ current: 0, total: 0, isRefreshing: false });
      }, 1000);
    }
  }, []);

  // 刷新所有账户
  const refreshAllAccounts = useCallback(async () => {
    if (!accountData?.accounts || accountData.accounts.length === 0) {
      return { success: false, message: "没有账户需要刷新" };
    }

    const totalAccounts = accountData.accounts.length;
    performanceMonitor.start('refreshAllAccounts');
    console.log(`🚀 开始批量刷新 ${totalAccounts} 个账户...`);
    
    setRefreshProgress({ current: 0, total: totalAccounts, isRefreshing: true });

    try {
      const { ConfigService } = await import("../services/configService");
      const accounts = accountData.accounts;
      let refreshedCount = 0;
      let successCount = 0;
      let tokenExpiredCount = 0;
      let networkErrorCount = 0;
      const updatedAccountsMap = new Map();

      const BATCH_SIZE = concurrentLimit;
      const batches: AccountInfo[][] = [];
      
      for (let i = 0; i < accounts.length; i += BATCH_SIZE) {
        batches.push(accounts.slice(i, i + BATCH_SIZE));
      }

      for (let batchIndex = 0; batchIndex < batches.length; batchIndex++) {
        const batch = batches[batchIndex];
        performanceMonitor.start(`refreshBatch-${batchIndex}`);
        
        const batchPromises = batch.map(async (account) => {
          try {
            const authResult = await ConfigService.refreshSingleAccountInfo(account.token);
            if (authResult.success && authResult.user_info?.account_info) {
              return {
                email: account.email,
                status: 'ok' as const,
                data: {
                  ...account,
                  subscription_type: authResult.user_info.account_info.subscription_type,
                  subscription_status: authResult.user_info.account_info.subscription_status,
                  trial_days_remaining: authResult.user_info.account_info.trial_days_remaining,
                },
              };
            }
            const apiStatus = authResult.user_info?.api_status;
            if (apiStatus === 401 || apiStatus === 403) {
              return { email: account.email, status: 'token_expired' as const, data: { ...account, subscription_type: "token_expired" } };
            }
            return { email: account.email, status: 'network_error' as const, data: account };
          } catch {
            return { email: account.email, status: 'network_error' as const, data: account };
          }
        });

        const batchResults = await Promise.allSettled(batchPromises);
        
        batchResults.forEach((result) => {
          if (result.status === 'fulfilled' && result.value) {
            const v = result.value;
            updatedAccountsMap.set(v.email, v.data);
            if (v.status === 'ok') successCount++;
            else if (v.status === 'token_expired') tokenExpiredCount++;
            else networkErrorCount++;
          }
          refreshedCount++;
        });

        const batchDuration = performanceMonitor.end(`refreshBatch-${batchIndex}`);
        console.log(`📦 批次 ${batchIndex + 1}/${batches.length} 完成，耗时: ${batchDuration.toFixed(2)}ms`);

        setRefreshProgress({ current: refreshedCount, total: totalAccounts, isRefreshing: true });

        setAccountData((prevData) => {
          if (!prevData?.accounts) return prevData;
          const updatedAccounts = prevData.accounts.map((acc) => 
            updatedAccountsMap.get(acc.email) || acc
          );
          return { ...prevData, accounts: updatedAccounts };
        });

        if (batchIndex < batches.length - 1) {
          // Increased delay to avoid API rate limiting
          await new Promise(resolve => setTimeout(resolve, 500));
        }
      }

      const finalAccounts = accounts.map((acc) =>
        updatedAccountsMap.get(acc.email) || acc
      );
      await ConfigService.saveAccountCache(finalAccounts);

      const failCount = tokenExpiredCount + networkErrorCount;
      const parts: string[] = [`成功 ${successCount}`];
      if (tokenExpiredCount > 0) parts.push(`Token 失效 ${tokenExpiredCount}`);
      if (networkErrorCount > 0) parts.push(`网络错误 ${networkErrorCount}`);
      const message = `刷新完成: ${parts.join('，')}`;

      return { success: failCount === 0, message };
    } catch (error) {
      console.error("刷新所有账户失败:", error);
      return { success: false, message: `刷新异常: ${error}` };
    } finally {
      const totalDuration = performanceMonitor.end('refreshAllAccounts');
      console.log(`✅ 批量刷新完成，总耗时: ${totalDuration.toFixed(2)}ms`);
      
      setTimeout(() => {
        setRefreshProgress({ current: 0, total: 0, isRefreshing: false });
      }, 1500);
    }
  }, [accountData, concurrentLimit]);

  // 添加账户到本地列表（不调用 API 获取订阅信息）
  const addAccountToList = useCallback(async (_email: string) => {
    try {
      // 重新加载账户列表，但只从本地文件读取，不获取订阅信息
      const result = await AccountService.getAccountList();

      if (result.success) {
        // ✅ 关键修复：合并新账号和已有账号的订阅信息
        // 保留已有账号的订阅信息，只添加新账号（订阅信息为空）
        const mergedAccounts = result.accounts.map((newAccount) => {
          // 查找当前状态中是否已有这个账号
          const existingAccount = accountData?.accounts.find(
            (acc) => acc.email === newAccount.email
          );

          if (existingAccount) {
            // ✅ 如果是已有账号，保留其订阅信息
            return {
              ...newAccount,
              subscription_type: existingAccount.subscription_type,
              subscription_status: existingAccount.subscription_status,
              trial_days_remaining: existingAccount.trial_days_remaining,
            };
          } else {
            // ✅ 如果是新账号，订阅信息为空（用户需要手动刷新）
            return newAccount;
          }
        });

        setAccountData({
          ...result,
          accounts: mergedAccounts,
        });
        return { success: true };
      }
      return { success: false, message: result.message };
    } catch (error) {
      console.error("Failed to add account to list:", error);
      return { success: false, message: "添加账户到列表失败" };
    }
  }, [accountData]);

  // 删除账户
  const removeAccount = useCallback(async (email: string) => {
    try {
      const result = await AccountService.removeAccount(email);
      if (result.success) {
        await loadAccounts();
        return { success: true };
      }
      return { success: false, message: result.message };
    } catch (error) {
      console.error("Failed to remove account:", error);
      return { success: false, message: "删除账户失败" };
    }
  }, [loadAccounts]);

  // 批量删除选中的账户
  const removeSelectedAccounts = useCallback(async () => {
    if (selectedAccounts.size === 0) {
      return { success: false, message: "没有选中的账户" };
    }

    try {
      const emails = Array.from(selectedAccounts);
      let successCount = 0;
      let failCount = 0;

      for (const email of emails) {
        const result = await AccountService.removeAccount(email);
        if (result.success) {
          successCount++;
        } else {
          failCount++;
        }
      }

      // 清空选中状态
      setSelectedAccounts(new Set());

      // 重新加载账户列表
      await loadAccounts();

      if (failCount === 0) {
        return { success: true, message: `成功删除 ${successCount} 个账户` };
      } else {
        return {
          success: true,
          message: `删除完成：成功 ${successCount} 个，失败 ${failCount} 个`
        };
      }
    } catch (error) {
      console.error("Failed to remove selected accounts:", error);
      return { success: false, message: "批量删除失败" };
    }
  }, [selectedAccounts, loadAccounts]);

  // 批量刷新选中的账户
  const refreshSelectedAccounts = useCallback(async () => {
    if (selectedAccounts.size === 0) {
      return { success: false, message: "没有选中的账户" };
    }

    try {
      const emails = Array.from(selectedAccounts);
      const total = emails.length;

      setRefreshProgress({ current: 0, total, isRefreshing: true });

      let successCount = 0;
      let tokenExpiredCount = 0;
      let networkErrorCount = 0;
      const updatedAccountsMap = new Map<string, AccountInfo>();

      for (let i = 0; i < emails.length; i += concurrentLimit) {
        const batch = emails.slice(i, i + concurrentLimit);

        await Promise.all(
          batch.map(async (email) => {
            try {
              const account = accountData?.accounts?.find(acc => acc.email === email);
              if (!account) {
                networkErrorCount++;
                return;
              }
              const result = await ConfigService.refreshSingleAccountInfo(account.token);
              if (result.success && result.user_info?.account_info) {
                successCount++;
                updatedAccountsMap.set(email, {
                  ...account,
                  subscription_type: result.user_info.account_info.subscription_type,
                  subscription_status: result.user_info.account_info.subscription_status,
                  trial_days_remaining: result.user_info.account_info.trial_days_remaining,
                });
              } else {
                const apiStatus = result.user_info?.api_status;
                if (apiStatus === 401 || apiStatus === 403) {
                  tokenExpiredCount++;
                  updatedAccountsMap.set(email, { ...account, subscription_type: "token_expired" });
                } else {
                  networkErrorCount++;
                }
              }
            } catch (error) {
              console.error(`Failed to refresh account ${email}:`, error);
              networkErrorCount++;
            }
          })
        );

        setRefreshProgress({
          current: Math.min(i + concurrentLimit, total),
          total,
          isRefreshing: true
        });
      }

      // 将刷新结果合并到 state 并保存到缓存
      if (accountData?.accounts) {
        const mergedAccounts = accountData.accounts.map((acc) =>
          updatedAccountsMap.get(acc.email) || acc
        );

        setAccountData((prev) => {
          if (!prev?.accounts) return prev;
          return { ...prev, accounts: mergedAccounts };
        });

        await ConfigService.saveAccountCache(mergedAccounts);
      }

      const failCount = tokenExpiredCount + networkErrorCount;
      const parts: string[] = [`成功 ${successCount}`];
      if (tokenExpiredCount > 0) parts.push(`Token 失效 ${tokenExpiredCount}`);
      if (networkErrorCount > 0) parts.push(`网络错误 ${networkErrorCount}`);
      const message = `刷新完成: ${parts.join('，')}`;

      return { success: failCount === 0, message };
    } catch (error) {
      console.error("Failed to refresh selected accounts:", error);
      return { success: false, message: "批量刷新失败" };
    } finally {
      setTimeout(() => {
        setRefreshProgress({ current: 0, total: 0, isRefreshing: false });
      }, 1500);
    }
  }, [selectedAccounts, accountData, concurrentLimit]);

  // 切换账户选择
  const toggleAccountSelection = useCallback((email: string) => {
    setSelectedAccounts((prev) => {
      const newSet = new Set(prev);
      if (newSet.has(email)) {
        newSet.delete(email);
      } else {
        newSet.add(email);
      }
      return newSet;
    });
  }, []);

  // 全选/取消全选
  const toggleSelectAll = useCallback(() => {
    if (!accountData?.accounts) return;
    
    if (selectedAccounts.size === accountData.accounts.length) {
      setSelectedAccounts(new Set());
    } else {
      setSelectedAccounts(new Set(accountData.accounts.map((acc) => acc.email)));
    }
  }, [accountData, selectedAccounts.size]);

  // 动态生成订阅类型筛选选项
  const subscriptionFilterOptions = useMemo(() => {
    const options = [{ value: "all", label: "全部账户" }];
    if (!accountData?.accounts) return options;

    const types = new Set<string>();
    for (const acc of accountData.accounts) {
      const t = acc.subscription_type;
      if (t) types.add(t);
    }

    const labelMap: Record<string, string> = {
      pro: "Pro", ultra: "Ultra", business: "Business",
      free_trial: "Trial", free: "Free",
    };

    for (const t of Array.from(types).sort()) {
      options.push({ value: t, label: labelMap[t] || t });
    }

    // 如果存在无订阅类型的账户，补一个 Free 选项
    if (accountData.accounts.some(a => !a.subscription_type) && !types.has("free")) {
      options.push({ value: "free", label: "Free" });
    }

    return options;
  }, [accountData]);

  // 动态生成标签筛选选项
  const tagFilterOptions = useMemo(() => {
    // Start with unstashed (default) and all options
    const options = [
      { value: "__unstashed__", label: "未隐藏" },  // Default: exclude stashed
      { value: "all", label: "全部标签" },          // Show everything including stashed
    ];
    if (!accountData?.accounts) return options;

    const tags = new Set<string>();
    for (const acc of accountData.accounts) {
      if (acc.tags) {
        for (const t of acc.tags) {
          // Don't show "stashed" as a selectable filter option (it's a system tag)
          if (t !== STASH_TAG) {
            tags.add(t);
          }
        }
      }
    }
    for (const t of Array.from(tags).sort()) {
      options.push({ value: t, label: t });
    }

    if (accountData.accounts.some(a => !a.tags || a.tags.length === 0)) {
      options.push({ value: "__untagged__", label: "未标记" });
    }

    return options;
  }, [accountData]);

  // 过滤并排序账户列表
  const filteredAccounts = useMemo(() => {
    if (!accountData?.accounts) return [];

    // 先过滤
    let result = accountData.accounts.filter((account) => {
      // 订阅类型过滤
      if (subscriptionFilter !== "all") {
        if (subscriptionFilter === "free") {
          if (account.subscription_type && account.subscription_type !== "free") return false;
        } else if (account.subscription_type !== subscriptionFilter) {
          return false;
        }
      }
      // 标签过滤
      if (tagFilter === "__unstashed__") {
        // Default filter: exclude stashed accounts
        if (account.tags?.includes(STASH_TAG)) return false;
      } else if (tagFilter !== "all") {
        if (tagFilter === "__untagged__") {
          // Show only accounts without tags (or only with stashed tag)
          const nonStashTags = account.tags?.filter(t => t !== STASH_TAG) || [];
          if (nonStashTags.length > 0) return false;
        } else {
          // Show accounts with specific tag
          if (!account.tags || !account.tags.includes(tagFilter)) return false;
        }
      }
      // When "all" is selected, show everything including stashed
      return true;
    });

    // 再排序
    if (sortField !== 'none') {
      result = [...result].sort((a, b) => {
        let comparison = 0;
        
        switch (sortField) {
          case 'date':
            // 按创建日期排序
            const dateA = new Date(a.created_at).getTime() || 0;
            const dateB = new Date(b.created_at).getTime() || 0;
            comparison = dateA - dateB;
            break;
          case 'trial':
            // 按试用剩余天数排序（无试用的放最后）
            const trialA = a.trial_days_remaining ?? -1;
            const trialB = b.trial_days_remaining ?? -1;
            comparison = trialA - trialB;
            break;
          case 'usage':
            // 按用量费用排序
            const usageA = a.usage_cost_cents ?? 0;
            const usageB = b.usage_cost_cents ?? 0;
            comparison = usageA - usageB;
            break;
          case 'email':
            // 按邮箱字母排序
            comparison = (a.email || '').localeCompare(b.email || '');
            break;
          case 'subscription':
            // 按订阅类型排序
            const subOrder: Record<string, number> = { 'ultra': 1, 'pro_plus': 2, 'pro': 3, 'business': 4, 'free_trial': 5, 'free': 6, 'token_expired': 7 };
            const subA = subOrder[a.subscription_type || 'free'] || 99;
            const subB = subOrder[b.subscription_type || 'free'] || 99;
            comparison = subA - subB;
            break;
          default:
            comparison = 0;
        }
        
        return sortOrder === 'asc' ? comparison : -comparison;
      });
    }

    return result;
  }, [accountData, subscriptionFilter, tagFilter, sortField, sortOrder]);

  // 排序选项
  const sortOptions = useMemo(() => [
    { value: 'none', label: '默认排序' },
    { value: 'date', label: '创建日期' },
    { value: 'trial', label: '试用天数' },
    { value: 'usage', label: '用量费用' },
    { value: 'email', label: '邮箱' },
    { value: 'subscription', label: '订阅类型' },
  ], []);

  // 更新排序并保存到本地存储和文件
  const updateSort = useCallback((field: string, order: 'asc' | 'desc') => {
    setSortField(field);
    setSortOrder(order);
    safeStorage.set('account_sort_field', field);
    safeStorage.set('account_sort_order', order);
    // 同步保存到文件供无感换号使用
    invoke('save_sort_settings', { field, order }).catch(err => {
      console.warn('Failed to save sort settings to file:', err);
    });
  }, []);

  // 更新单个账户的用量费用
  const updateAccountUsageCost = useCallback((email: string, usageCostCents: number) => {
    setAccountData((prev) => {
      if (!prev?.accounts) return prev;
      const updatedAccounts = prev.accounts.map((acc) =>
        acc.email === email ? { ...acc, usage_cost_cents: usageCostCents } : acc
      );
      return { ...prev, accounts: updatedAccounts };
    });
  }, []);

  // 刷新所有账户的用量数据（从API获取）
  const refreshAllAccountsUsage = useCallback(async () => {
    if (!accountData?.accounts || accountData.accounts.length === 0) {
      return { success: false, message: "没有账户需要刷新用量" };
    }

    const totalAccounts = accountData.accounts.length;
    performanceMonitor.start('refreshAllAccountsUsage');
    console.log(`🚀 开始批量获取 ${totalAccounts} 个账户的用量数据...`);

    setRefreshProgress({ current: 0, total: totalAccounts, isRefreshing: true });

    try {
      const accounts = accountData.accounts;
      let refreshedCount = 0;
      let successCount = 0;
      const updatedCostsMap = new Map<string, number>();

      // 默认日期范围: 02/01 - 05/01
      const now = new Date();
      const year = now.getFullYear();
      const startDate = new Date(year, 1, 1).getTime(); // 2月1日
      const endDate = new Date(year, 4, 1).getTime();   // 5月1日

      const BATCH_SIZE = concurrentLimit;
      const batches: AccountInfo[][] = [];

      for (let i = 0; i < accounts.length; i += BATCH_SIZE) {
        batches.push(accounts.slice(i, i + BATCH_SIZE));
      }

      for (let batchIndex = 0; batchIndex < batches.length; batchIndex++) {
        const batch = batches[batchIndex];
        performanceMonitor.start(`refreshUsageBatch-${batchIndex}`);

        const batchPromises = batch.map(async (account) => {
          try {
            // 跳过 token 失效的账户
            if (account.subscription_type === "token_expired") {
              return { email: account.email, status: 'skipped' as const, cost: 0 };
            }

            const result = await AccountUsageService.getAccountUsageAndSave(
              account,
              startDate,
              endDate,
              0
            );

            if (result.success && result.data) {
              const totalCost = result.data.aggregatedData.total_cost_cents || 0;
              return { email: account.email, status: 'ok' as const, cost: totalCost };
            }
            // 如果没有用量数据，返回 $0.00 而不是错误
            return { email: account.email, status: 'ok' as const, cost: 0 };
          } catch (error) {
            console.error(`获取 ${account.email} 用量失败:`, error);
            // 即使请求失败，也返回 $0.00 而不是错误
            return { email: account.email, status: 'ok' as const, cost: 0 };
          }
        });

        const batchResults = await Promise.allSettled(batchPromises);

        batchResults.forEach((result) => {
          if (result.status === 'fulfilled' && result.value) {
            const v = result.value;
            if (v.status === 'ok') {
              successCount++;
              updatedCostsMap.set(v.email, v.cost);
            }
            // 'skipped' status (token_expired) 不计入成功或失败
          }
          refreshedCount++;
        });

        const batchDuration = performanceMonitor.end(`refreshUsageBatch-${batchIndex}`);
        console.log(`📦 用量批次 ${batchIndex + 1}/${batches.length} 完成，耗时: ${batchDuration.toFixed(2)}ms`);

        setRefreshProgress({ current: refreshedCount, total: totalAccounts, isRefreshing: true });

        // 实时更新账户费用
        setAccountData((prevData) => {
          if (!prevData?.accounts) return prevData;
          const updatedAccounts = prevData.accounts.map((acc) => {
            const newCost = updatedCostsMap.get(acc.email);
            return newCost !== undefined ? { ...acc, usage_cost_cents: newCost } : acc;
          });
          return { ...prevData, accounts: updatedAccounts };
        });

        if (batchIndex < batches.length - 1) {
          await new Promise(resolve => setTimeout(resolve, 100));
        }
      }

      const message = `用量刷新完成: 成功 ${successCount} 个账户`;
      console.log(`✅ ${message}`);

      return { success: true, message };
    } catch (error) {
      console.error("刷新所有账户用量失败:", error);
      return { success: false, message: `刷新用量异常: ${error}` };
    } finally {
      const totalDuration = performanceMonitor.end('refreshAllAccountsUsage');
      console.log(`✅ 批量用量刷新完成，总耗时: ${totalDuration.toFixed(2)}ms`);

      setTimeout(() => {
        setRefreshProgress({ current: 0, total: 0, isRefreshing: false });
      }, 1500);
    }
  }, [accountData, concurrentLimit]);

  // Check if an account is stashed
  const isStashed = useCallback((account: AccountInfo) => {
    return account.tags?.includes(STASH_TAG) ?? false;
  }, []);

  // Stash an account (add stashed tag)
  const stashAccount = useCallback(async (email: string) => {
    try {
      const account = accountData?.accounts.find(a => a.email === email);
      if (!account) return { success: false, message: "账户不存在" };

      // Add stashed tag if not already present
      const currentTags = account.tags || [];
      if (currentTags.includes(STASH_TAG)) {
        return { success: true, message: "账户已经被隐藏" };
      }

      const newTags = [...currentTags, STASH_TAG];
      const result = await AccountService.editAccount(
        email,
        undefined, // newEmail
        undefined, // newToken
        undefined, // newRefreshToken
        undefined, // newWorkosSessionToken
        undefined, // newUsername
        newTags    // newTags
      );

      if (result.success) {
        // Update local state
        setAccountData((prevData) => {
          if (!prevData?.accounts) return prevData;
          const updatedAccounts = prevData.accounts.map((acc) =>
            acc.email === email ? { ...acc, tags: newTags } : acc
          );
          ConfigService.saveAccountCache(updatedAccounts);
          return { ...prevData, accounts: updatedAccounts };
        });
      }

      return result;
    } catch (error) {
      console.error("隐藏账户失败:", error);
      return { success: false, message: `隐藏失败: ${error}` };
    }
  }, [accountData]);

  // Unstash an account (remove stashed tag)
  const unstashAccount = useCallback(async (email: string) => {
    try {
      const account = accountData?.accounts.find(a => a.email === email);
      if (!account) return { success: false, message: "账户不存在" };

      // Remove stashed tag
      const currentTags = account.tags || [];
      if (!currentTags.includes(STASH_TAG)) {
        return { success: true, message: "账户未被隐藏" };
      }

      const newTags = currentTags.filter(t => t !== STASH_TAG);
      const result = await AccountService.editAccount(
        email,
        undefined, // newEmail
        undefined, // newToken
        undefined, // newRefreshToken
        undefined, // newWorkosSessionToken
        undefined, // newUsername
        newTags    // newTags
      );

      if (result.success) {
        // Update local state
        setAccountData((prevData) => {
          if (!prevData?.accounts) return prevData;
          const updatedAccounts = prevData.accounts.map((acc) =>
            acc.email === email ? { ...acc, tags: newTags } : acc
          );
          ConfigService.saveAccountCache(updatedAccounts);
          return { ...prevData, accounts: updatedAccounts };
        });
      }

      return result;
    } catch (error) {
      console.error("取消隐藏账户失败:", error);
      return { success: false, message: `取消隐藏失败: ${error}` };
    }
  }, [accountData]);

  return {
    accountData,
    loading,
    selectedAccounts,
    subscriptionFilter,
    refreshProgress,
    concurrentLimit,
    filteredAccounts,
    subscriptionFilterOptions,
    tagFilter,
    tagFilterOptions,
    sortField,
    sortOrder,
    sortOptions,
    loadAccounts,
    refreshCurrentAccount,
    addAccountToList,
    refreshSingleAccount,
    refreshAllAccounts,
    refreshAllAccountsUsage,
    removeAccount,
    removeSelectedAccounts,
    refreshSelectedAccounts,
    toggleAccountSelection,
    toggleSelectAll,
    setSubscriptionFilter,
    setTagFilter,
    setConcurrentLimit,
    updateAccountUsageCost,
    updateSort,
    // Stash functions
    isStashed,
    stashAccount,
    unstashAccount,
  };
};


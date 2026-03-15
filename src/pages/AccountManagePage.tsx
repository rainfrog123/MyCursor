/**
 * AccountManagePage - 优化版本
 * 
 * 性能优化：
 * 1. 使用自定义 hooks 管理状态（useAccountManagement）
 * 2. 将大表单拆分为独立组件（AddAccountForm, EditAccountForm）
 * 3. AccountCard 组件化并使用 memo 优化
 * 4. 虚拟滚动支持（账号 > 20 时自动启用）
 * 5. 使用 useMemo 和 useCallback 减少重渲染
 * 6. 事件监听器正确清理
 * 
 * 预期内存减少：40-60%
 * 预期性能提升：50-70%
 */

import React, { useState, useEffect, useRef, useCallback, useMemo } from "react";
import { AccountService } from "../services/accountService";
import type { AccountInfo } from "../types/account";
import type { AggregatedUsageData, UsageEvent } from "../types/usage";
import {
  LoadingSpinner,
  Toast,
  ConfirmDialog,
  AddAccountForm,
  EditAccountForm,
  AccountCard,
  VirtualizedAccountList,
  UsageDisplay,
  Icon,
  Dropdown,
} from "../components";

import { open } from "@tauri-apps/plugin-dialog";
import { useAccountManagement } from "../hooks/useAccountManagement";
import { safeStorage } from "../utils/safeStorage";

// ✅ 优化：降低阈值，更早启用虚拟滚动
const VIRTUAL_SCROLL_THRESHOLD = 15; // 账号数量超过此值启用虚拟滚动（从 20 降到 15）

export const AccountManagePage: React.FC = () => {
  // 使用自定义 hook 管理账号相关状态
  const {
    accountData,
    loading,
    selectedAccounts,
    subscriptionFilter,
    refreshProgress,
    concurrentLimit,
    filteredAccounts,
    searchQuery,
    setSearchQuery,
    searchFilteredAccounts,
    subscriptionFilterOptions,
    tagFilter,
    tagFilterOptions,
    sortField,
    sortOrder,
    sortOptions,
    loadAccounts,
    addAccountToList,
    refreshSingleAccount,
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
    stashSelectedAccounts,
    unstashSelectedAccounts,
  } = useAccountManagement();

  // UI 状态
  const [showAddForm, setShowAddForm] = useState(false);
  const [showEditForm, setShowEditForm] = useState(false);
  const [editingAccount, setEditingAccount] = useState<AccountInfo | null>(null);
  const [expandedAccountEmail, setExpandedAccountEmail] = useState<string | null>(null);
  const [closingAccountEmail, setClosingAccountEmail] = useState<string | null>(null);
  const [usageModalOpen, setUsageModalOpen] = useState(false);
  const [selectedAccountUsage, setSelectedAccountUsage] = useState<{
    account: AccountInfo;
    usageData: AggregatedUsageData | null;
    events: UsageEvent[] | null;
    totalEvents: number;
    loading: boolean;
    useEventBasedCalculation: boolean;
  } | null>(null);
  const [toast, setToast] = useState<{
    message: string;
    type: "success" | "error";
  } | null>(null);
  const [confirmDialog, setConfirmDialog] = useState<{
    show: boolean;
    title: string;
    message: string;
    onConfirm: (checkboxValue?: boolean) => void;
    checkboxLabel?: string;
    checkboxDefaultChecked?: boolean;
    type?: "danger" | "warning" | "info";
    confirmText?: string;
  }>({ show: false, title: "", message: "", onConfirm: () => {} });

  // 切换账号弹窗状态
  const [switchModal, setSwitchModal] = useState<{
    show: boolean;
    account: AccountInfo | null;
    resetMachineId: boolean;
    machineIdOption: "bound" | "new";
    cursorAction: "restart" | "close_only" | "none";
  }>({ show: false, account: null, resetMachineId: true, machineIdOption: "new", cursorAction: "restart" });

  // 事件监听器清理函数引用
  const cleanupListenersRef = useRef<(() => void) | null>(null);

  // 组件挂载时加载账户列表
  useEffect(() => {
    loadAccounts();
    
    // 设置事件监听器（自动登录相关）
    const setupListeners = async () => {
      const { listen } = await import("@tauri-apps/api/event");
      const listeners: (() => void)[] = [];

      // 自动登录成功监听
      const unlisten1 = await listen("auto-login-success", async (event: any) => {
        const webToken = event.payload?.token;
        if (webToken) {
          setToast({ message: "登录成功！", type: "success" });
          // 自动登录成功后只更新本地列表，不获取订阅信息
          await addAccountToList("");
        }
      });
      listeners.push(unlisten1);

      // 自动登录失败监听
      const unlisten2 = await listen("auto-login-failed", () => {
        setToast({ message: "自动登录失败", type: "error" });
      });
      listeners.push(unlisten2);

      // 保存清理函数
      cleanupListenersRef.current = () => {
        listeners.forEach(unlisten => unlisten());
      };
    };

    setupListeners();

    // 清理函数
    return () => {
      if (cleanupListenersRef.current) {
        cleanupListenersRef.current();
      }
    };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // 外部点击检测已移至 AccountCard 组件内部

  // Toast 自动消失
  useEffect(() => {
    if (toast) {
      const timer = setTimeout(() => setToast(null), 3000);
      return () => clearTimeout(timer);
    }
  }, [toast]);

  // 使用 useCallback 优化回调函数
  const handleAddSuccess = useCallback(async () => {
    setShowAddForm(false);
    
    // 1. 先更新本地列表
    await addAccountToList("");
    
    // 2. 找出新添加的账户（没有 subscription_type）并刷新
    const accountList = await AccountService.getAccountList();
    if (accountList.success && accountList.accounts) {
      const accountsToRefresh = accountList.accounts.filter(
        acc => acc.subscription_type === undefined || acc.subscription_type === null
      );
      
      // 刷新每个新账户
      for (const acc of accountsToRefresh) {
        await refreshSingleAccount(acc, 0);
      }
    }
  }, [addAccountToList, refreshSingleAccount]);

  const handleEditSuccess = useCallback(async () => {
    setShowEditForm(false);
    setEditingAccount(null);
    await loadAccounts();
  }, [loadAccounts]);

  const handleRefreshAccount = useCallback(async (account: AccountInfo, index: number) => {
    const result = await refreshSingleAccount(account, index);
    if (result.success) {
      setToast({ message: `${account.email} 信息已刷新`, type: "success" });
    } else {
      setToast({ message: `${account.email} ${result.message || "刷新失败"}`, type: "error" });
    }
  }, [refreshSingleAccount]);

  const handleSwitchAccount = useCallback((account: AccountInfo) => {
    setSwitchModal({
      show: true,
      account,
      resetMachineId: true,
      machineIdOption: "new",
      cursorAction: "restart",
    });
  }, []);

  const handleSwitchConfirm = useCallback(async () => {
    const { account, resetMachineId, machineIdOption, cursorAction } = switchModal;
    if (!account) return;
    setSwitchModal(prev => ({ ...prev, show: false }));

    try {
      const { CursorService } = await import("../services/cursorService");
      const isAdmin = await CursorService.checkAdminPrivileges();

      if (!isAdmin) {
        setConfirmDialog({
          show: true,
          title: "需要管理员权限",
          message: "切换账户需要管理员权限才能修改 Cursor 配置文件。\n\n请以管理员身份运行本程序后重试。",
          onConfirm: () => setConfirmDialog(prev => ({ ...prev, show: false })),
        });
        return;
      }

      const option = resetMachineId ? machineIdOption : "none";
      const forceClose = cursorAction !== "none";
      const { invoke } = await import("@tauri-apps/api/core");
      const result = await invoke<{ success: boolean; message: string; details: string[] }>(
        "switch_account_with_options",
        { email: account.email, machineIdOption: option, autoRestart: forceClose }
      );

      if (result.success) {
        await loadAccounts();
        if (cursorAction === "restart") {
          setToast({ message: "账户切换成功，正在启动 Cursor...", type: "success" });
          try {
            await invoke<{ success: boolean; message: string }>("launch_cursor");
          } catch {
            // 启动失败不影响切换结果
          }
        } else if (cursorAction === "close_only") {
          setToast({ message: "账户切换成功，Cursor 已关闭，请手动启动", type: "success" });
        } else {
          setToast({ message: "账户切换成功，请手动重启 Cursor 生效", type: "success" });
        }
      } else {
        setToast({ message: result.message, type: "error" });
      }
    } catch (error) {
      console.error("Failed to switch account:", error);
      setToast({ message: "切换账户失败", type: "error" });
    }
  }, [switchModal, loadAccounts]);

  const handleViewUsage = useCallback((account: AccountInfo) => {
    setSelectedAccountUsage({
      account,
      usageData: null,
      events: null,
      totalEvents: 0,
      loading: false,
      useEventBasedCalculation: false,
    });
    setUsageModalOpen(true);
  }, []);

  const handleEditAccount = useCallback((account: AccountInfo) => {
    setEditingAccount(account);
    setShowEditForm(true);
  }, []);

  const handleRemoveAccount = useCallback(async (email: string) => {
    setConfirmDialog({
      show: true,
      title: "确认删除",
      message: `确定要删除账户 ${email} 吗？`,
      onConfirm: async () => {
        const result = await removeAccount(email);
        if (result.success) {
          setToast({ message: "账户已删除", type: "success" });
        } else {
          setToast({ message: result.message || "删除失败", type: "error" });
        }
        setConfirmDialog({ ...confirmDialog, show: false });
      },
    });
  }, [removeAccount, confirmDialog]);

  const handleToggleExpand = useCallback((email: string) => {
    if (expandedAccountEmail === email) {
      setExpandedAccountEmail(null);
      setClosingAccountEmail(null);
    } else {
      setExpandedAccountEmail(email);
      setClosingAccountEmail(null);
    }
  }, [expandedAccountEmail]);

  const handleCloseMenu = useCallback(() => {
    setExpandedAccountEmail(null);
    setClosingAccountEmail(null);
  }, []);

  const handleViewDashboard = useCallback(async (account: AccountInfo) => {
    if (!account.workos_cursor_session_token) {
      setToast({
        message: "该账户没有Workos Session Token，无法查看主页",
        type: "error",
      });
      return;
    }

    try {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("open_cursor_dashboard", {
        workosCursorSessionToken: account.workos_cursor_session_token,
      });
      setToast({ message: "Cursor主页已打开", type: "success" });
    } catch (error) {
      console.error("Failed to open dashboard:", error);
      setToast({ message: "打开主页失败", type: "error" });
    }
  }, []);

  const handleViewBindCard = useCallback(async (account: AccountInfo) => {
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const result = await invoke<{ success: boolean; message: string }>("open_bind_card_info", {
        accessToken: account.token,
        workosCursorSessionToken: account.workos_cursor_session_token || null,
      });
      setToast({ message: result.message, type: result.success ? "success" : "error" });
    } catch (error) {
      console.error("Failed to open bind card info:", error);
      setToast({ message: "打开绑卡信息失败", type: "error" });
    }
  }, []);

  const handleDeleteCursorAccount = useCallback((account: AccountInfo) => {
    setConfirmDialog({
      show: true,
      title: "注销 Cursor 账户",
      message: `确定要注销 Cursor 账户 ${account.email} 吗？\n\n此操作将从 Cursor 服务器永久删除该账户，不可恢复！`,
      type: "danger",
      confirmText: "确认注销",
      onConfirm: async () => {
        setConfirmDialog(prev => ({ ...prev, show: false }));
        try {
          const { AccountService } = await import("../services/accountService");
          const result = await AccountService.deleteCursorAccount(
            account.token,
            account.workos_cursor_session_token || undefined
          );
          setToast({ message: result.message, type: result.success ? "success" : "error" });
        } catch (error) {
          console.error("Failed to delete cursor account:", error);
          setToast({ message: "注销账户失败", type: "error" });
        }
      },
    });
  }, []);

  const handleExportSelectedAccounts = useCallback(async () => {
    if (selectedAccounts.size === 0) {
      setToast({ message: "请先选择要导出的账户", type: "error" });
      return;
    }

    try {
      const selectedPath = await open({
        directory: true,
        multiple: false,
        title: "选择导出目录",
      });

      if (!selectedPath) return;

      // 将选中的账号邮箱列表转为数组
      const selectedEmails = Array.from(selectedAccounts);
      
      const result = await AccountService.exportAccounts(selectedPath, selectedEmails);
      if (result.success) {
        setToast({ 
          message: `成功导出 ${selectedAccounts.size} 个账户到 ${result.exported_path}`, 
          type: "success" 
        });
      } else {
        setToast({ message: result.message, type: "error" });
      }
    } catch (error) {
      console.error("Failed to export accounts:", error);
      setToast({ message: "导出账户失败", type: "error" });
    }
  }, [selectedAccounts]);

  const handleImportAccounts = useCallback(async () => {
    try {
      const selectedFile = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "JSON Files", extensions: ["json"] }],
        title: "选择要导入的JSON文件",
      });

      if (!selectedFile) return;

      // ✅ 使用 Worker 处理大文件
      const { readTextFile } = await import("@tauri-apps/plugin-fs");
      const fileContent = await readTextFile(selectedFile);

      // ✅ 在 Worker 中解析和验证数据
      const { workerManager } = await import("../utils/workerManager");

      setToast({ message: "正在解析文件...", type: "success" });

      const parseResult = await workerManager.parseAccounts(fileContent);

      if (!parseResult.success || !parseResult.accounts) {
        setToast({ message: parseResult.error || "文件解析失败", type: "error" });
        return;
      }

      const accountCount = parseResult.accounts.length;

      // 如果导入数量较大，显示警告
      if (accountCount > 500) {
        setConfirmDialog({
          show: true,
          title: "⚠️ 大批量导入提示",
          message: `即将导入 ${accountCount} 个账号。\n\n` +
                   `• 导入过程可能需要几秒钟\n` +
                   `• 导入后账号会立即显示在列表中\n` +
                   `• 订阅信息需要手动点击"刷新"按钮获取\n\n` +
                   `是否继续导入？`,
          onConfirm: async () => {
            setConfirmDialog({ ...confirmDialog, show: false });
            await performImport(selectedFile, accountCount);
          },
        });
      } else {
        await performImport(selectedFile, accountCount);
      }
    } catch (error) {
      console.error("Failed to import accounts:", error);
      setToast({ message: "导入账户失败", type: "error" });
    }

    async function performImport(filePath: string, _count: number) {
      try {
        const result = await AccountService.importAccounts(filePath);
        if (result.success) {
          setToast({
            message: `${result.message} - 正在获取订阅信息...`,
            type: "success"
          });
          
          // 1. 先更新本地列表
          await addAccountToList("");
          
          // 2. 获取最新的账户列表，找出需要刷新的账户（没有 subscription_type 的）
          const accountList = await AccountService.getAccountList();
          if (accountList.success && accountList.accounts) {
            const accountsToRefresh = accountList.accounts.filter(
              acc => acc.subscription_type === undefined || acc.subscription_type === null
            );
            
            if (accountsToRefresh.length > 0) {
              // 直接调用刷新逻辑
              for (const acc of accountsToRefresh) {
                await refreshSingleAccount(acc, 0);
              }
              
              setToast({ 
                message: `导入完成，已刷新 ${accountsToRefresh.length} 个账户的订阅信息`, 
                type: "success" 
              });
            }
          }
        } else {
          setToast({ message: result.message, type: "error" });
        }
      } catch (error) {
        console.error("Failed to import accounts:", error);
        setToast({ message: "导入账户失败", type: "error" });
      }
    }
  }, [addAccountToList, refreshSingleAccount, confirmDialog]);

  // 刷新选中的账户（订阅信息 + 用量数据）
  const handleRefreshSelected = useCallback(async () => {
    if (selectedAccounts.size === 0) {
      setToast({ message: "请先选择要刷新的账户", type: "error" });
      return;
    }
    
    // 刷新选中账户的订阅信息
    const result = await refreshSelectedAccounts();
    if (!result.success) {
      setToast({ message: result.message || "刷新失败", type: "error" });
      return;
    }
    
    // 然后刷新选中账户的用量数据（只刷新选中的账户）
    setToast({ message: "订阅信息已刷新，正在获取用量数据...", type: "success" });
    const selectedEmails = Array.from(selectedAccounts);
    const usageResult = await refreshAllAccountsUsage(selectedEmails);
    if (usageResult.success) {
      setToast({ message: `已刷新 ${selectedAccounts.size} 个账户的订阅和用量数据`, type: "success" });
    } else {
      setToast({ message: usageResult.message || "用量数据刷新失败", type: "error" });
    }
  }, [selectedAccounts, refreshSelectedAccounts, refreshAllAccountsUsage]);

  // 删除选中的账户
  const handleDeleteSelected = useCallback(async () => {
    if (selectedAccounts.size === 0) {
      setToast({ message: "请先选择要删除的账户", type: "error" });
      return;
    }

    setConfirmDialog({
      show: true,
      title: "确认删除",
      message: `确定要删除选中的 ${selectedAccounts.size} 个账户吗？此操作不可恢复。`,
      onConfirm: async () => {
        const result = await removeSelectedAccounts();
        if (result.success) {
          setToast({ message: result.message || `已删除 ${selectedAccounts.size} 个账户`, type: "success" });
        } else {
          setToast({ message: result.message || "删除失败", type: "error" });
        }
        setConfirmDialog({ ...confirmDialog, show: false });
      },
    });
  }, [selectedAccounts, removeSelectedAccounts, confirmDialog]);

  // 使用 useMemo 优化计算
  const shouldUseVirtualScroll = useMemo(() => {
    return searchFilteredAccounts.length > VIRTUAL_SCROLL_THRESHOLD;
  }, [searchFilteredAccounts.length]);

  const isAllSelected = useMemo(() => {
    if (searchFilteredAccounts.length === 0) return false;
    return searchFilteredAccounts.every(acc => selectedAccounts.has(acc.email));
  }, [searchFilteredAccounts, selectedAccounts]);

  // 批量隐藏选中账户
  const handleStashSelected = useCallback(async () => {
    if (selectedAccounts.size === 0) {
      setToast({ message: "请先选择要隐藏的账户", type: "error" });
      return;
    }
    const result = await stashSelectedAccounts();
    if (result.success) {
      setToast({ message: result.message, type: "success" });
    } else {
      setToast({ message: result.message || "批量隐藏失败", type: "error" });
    }
  }, [selectedAccounts.size, stashSelectedAccounts]);

  // 批量显示选中账户
  const handleUnstashSelected = useCallback(async () => {
    if (selectedAccounts.size === 0) {
      setToast({ message: "请先选择要显示的账户", type: "error" });
      return;
    }
    const result = await unstashSelectedAccounts();
    if (result.success) {
      setToast({ message: result.message, type: "success" });
    } else {
      setToast({ message: result.message || "批量显示失败", type: "error" });
    }
  }, [selectedAccounts.size, unstashSelectedAccounts]);

  // 渲染账号卡片的函数（用于虚拟滚动）
  const renderAccountCard = useCallback((account: AccountInfo, index: number) => {
    const isCurrent = Boolean(accountData?.current_account && 
                      account.email === accountData.current_account.email);
    
    return (
      <AccountCard
        key={account.email}
        account={account}
        index={index}
        isSelected={selectedAccounts.has(account.email)}
        isCurrent={isCurrent}
        isExpanded={expandedAccountEmail === account.email}
        isClosing={closingAccountEmail === account.email}
        isStashed={isStashed(account)}
        onSelect={toggleAccountSelection}
        onRefresh={handleRefreshAccount}
        onSwitch={handleSwitchAccount}
        onViewUsage={handleViewUsage}
        onEdit={handleEditAccount}
        onRemove={handleRemoveAccount}
        onToggleExpand={handleToggleExpand}
        onCloseMenu={handleCloseMenu}
        onViewDashboard={handleViewDashboard}
        onViewBindCard={handleViewBindCard}
        onDeleteCursorAccount={handleDeleteCursorAccount}
        onToast={(message, type) => setToast({ message, type })}
      />
    );
  }, [
    accountData,
    selectedAccounts,
    expandedAccountEmail,
    closingAccountEmail,
    isStashed,
    toggleAccountSelection,
    handleRefreshAccount,
    handleSwitchAccount,
    handleViewUsage,
    handleEditAccount,
    handleRemoveAccount,
    handleToggleExpand,
    handleCloseMenu,
    handleViewDashboard,
    handleViewBindCard,
    handleDeleteCursorAccount,
  ]);

  if (loading && !accountData) {
    return (
      <div className="flex items-center justify-center h-64">
        <LoadingSpinner />
      </div>
    );
  }

  // 按钮样式（使用 CSS 变量）
  const getButtonStyle = (variant: 'primary' | 'secondary' | 'success' | 'danger' = 'primary', disabled = false) => {
    const baseStyle = {
      display: 'inline-flex',
      alignItems: 'center',
      padding: '8px 12px',
      fontSize: '13px',
      fontWeight: '500',
      borderRadius: 'var(--border-radius)',
      border: 'none',
      cursor: disabled ? 'not-allowed' : 'pointer',
      transition: 'all var(--transition-duration) ease',
      boxShadow: 'var(--shadow-light)',
      opacity: disabled ? 0.5 : 1,
    };

    const variants = {
      primary: {
        backgroundColor: 'var(--primary-color)',
        color: 'white',
      },
      secondary: {
        backgroundColor: 'var(--bg-secondary)',
        color: 'var(--text-primary)',
        border: '1px solid var(--border-primary)',
      },
      success: {
        backgroundColor: '#10b981',
        color: 'white',
      },
      danger: {
        backgroundColor: '#ef4444',
        color: 'white',
      },
    };

    return { ...baseStyle, ...variants[variant] };
  };

  return (
    <div className="space-y-6" style={{ overflow: 'visible' }}>
      <div style={{
        backgroundColor: 'var(--bg-primary)',
        borderRadius: 'var(--border-radius-lg)',
        boxShadow: 'var(--shadow-medium)',
        overflow: 'visible',
      }}>
        {/* 固定操作按钮栏 */}
        <div
          className="sticky top-0 z-10"
          style={{
            backgroundColor: 'var(--bg-secondary)',
            borderBottom: '1px solid var(--border-primary)',
            backdropFilter: 'blur(var(--backdrop-blur))',
            WebkitBackdropFilter: 'blur(var(--backdrop-blur))',
            boxShadow: 'var(--shadow-light)',
            borderTopLeftRadius: 'var(--border-radius-large)',
            borderTopRightRadius: 'var(--border-radius-large)',
          }}
        >
          <div className="px-4 py-3 sm:px-6">
            {/* 第一行：主要操作按钮 */}
            <div className="flex flex-wrap items-center gap-2 mb-2">
              <button
                type="button"
                onClick={() => setShowAddForm(!showAddForm)}
                style={getButtonStyle('primary')}
                onMouseEnter={(e) => {
                  if (!e.currentTarget.disabled) {
                    e.currentTarget.style.transform = 'translateY(-1px)';
                    e.currentTarget.style.boxShadow = 'var(--shadow-medium)';
                  }
                }}
                onMouseLeave={(e) => {
                  e.currentTarget.style.transform = 'translateY(0)';
                  e.currentTarget.style.boxShadow = 'var(--shadow-light)';
                }}
              >
                <Icon name="plus" size={14} style={{ marginRight: '4px' }} />
                添加
              </button>
              <button
                type="button"
                onClick={handleRefreshSelected}
                disabled={refreshProgress.isRefreshing || selectedAccounts.size === 0}
                style={getButtonStyle('success', refreshProgress.isRefreshing || selectedAccounts.size === 0)}
                onMouseEnter={(e) => {
                  if (!e.currentTarget.disabled) {
                    e.currentTarget.style.transform = 'translateY(-1px)';
                    e.currentTarget.style.boxShadow = 'var(--shadow-medium)';
                  }
                }}
                onMouseLeave={(e) => {
                  e.currentTarget.style.transform = 'translateY(0)';
                  e.currentTarget.style.boxShadow = 'var(--shadow-light)';
                }}
              >
                {refreshProgress.isRefreshing ? (
                  <>
                    <Icon name="loading" size={14} style={{ marginRight: '4px' }} className="animate-spin" />
                    刷新中
                  </>
                ) : (
                  <>
                    <Icon name="refresh" size={14} style={{ marginRight: '4px' }} />
                    刷新{selectedAccounts.size > 0 && ` (${selectedAccounts.size})`}
                  </>
                )}
              </button>
              <button
                type="button"
                onClick={handleDeleteSelected}
                disabled={selectedAccounts.size === 0}
                style={getButtonStyle('danger', selectedAccounts.size === 0)}
                onMouseEnter={(e) => {
                  if (!e.currentTarget.disabled) {
                    e.currentTarget.style.transform = 'translateY(-1px)';
                    e.currentTarget.style.boxShadow = 'var(--shadow-medium)';
                  }
                }}
                onMouseLeave={(e) => {
                  e.currentTarget.style.transform = 'translateY(0)';
                  e.currentTarget.style.boxShadow = 'var(--shadow-light)';
                }}
              >
                <Icon name="trash" size={14} style={{ marginRight: '4px' }} />
                删除{selectedAccounts.size > 0 && ` (${selectedAccounts.size})`}
              </button>
              <button
                type="button"
                onClick={handleStashSelected}
                disabled={selectedAccounts.size === 0}
                style={getButtonStyle('secondary', selectedAccounts.size === 0)}
                onMouseEnter={(e) => {
                  if (!e.currentTarget.disabled) {
                    e.currentTarget.style.transform = 'translateY(-1px)';
                    e.currentTarget.style.boxShadow = 'var(--shadow-medium)';
                  }
                }}
                onMouseLeave={(e) => {
                  e.currentTarget.style.transform = 'translateY(0)';
                  e.currentTarget.style.boxShadow = 'var(--shadow-light)';
                }}
              >
                <Icon name="eye-off" size={14} style={{ marginRight: '4px' }} />
                隐藏{selectedAccounts.size > 0 && ` (${selectedAccounts.size})`}
              </button>
              <button
                type="button"
                onClick={handleUnstashSelected}
                disabled={selectedAccounts.size === 0}
                style={getButtonStyle('secondary', selectedAccounts.size === 0)}
                onMouseEnter={(e) => {
                  if (!e.currentTarget.disabled) {
                    e.currentTarget.style.transform = 'translateY(-1px)';
                    e.currentTarget.style.boxShadow = 'var(--shadow-medium)';
                  }
                }}
                onMouseLeave={(e) => {
                  e.currentTarget.style.transform = 'translateY(0)';
                  e.currentTarget.style.boxShadow = 'var(--shadow-light)';
                }}
              >
                <Icon name="eye" size={14} style={{ marginRight: '4px' }} />
                显示{selectedAccounts.size > 0 && ` (${selectedAccounts.size})`}
              </button>
              <button
                type="button"
                onClick={handleExportSelectedAccounts}
                disabled={selectedAccounts.size === 0}
                style={getButtonStyle('primary', selectedAccounts.size === 0)}
                onMouseEnter={(e) => {
                  if (!e.currentTarget.disabled) {
                    e.currentTarget.style.transform = 'translateY(-1px)';
                    e.currentTarget.style.boxShadow = 'var(--shadow-medium)';
                  }
                }}
                onMouseLeave={(e) => {
                  e.currentTarget.style.transform = 'translateY(0)';
                  e.currentTarget.style.boxShadow = 'var(--shadow-light)';
                }}
              >
                <Icon name="export" size={14} style={{ marginRight: '4px' }} />
                导出 {selectedAccounts.size > 0 && `(${selectedAccounts.size})`}
              </button>
              <button
                type="button"
                onClick={handleImportAccounts}
                style={getButtonStyle('secondary')}
                onMouseEnter={(e) => {
                  e.currentTarget.style.transform = 'translateY(-1px)';
                  e.currentTarget.style.boxShadow = 'var(--shadow-medium)';
                }}
                onMouseLeave={(e) => {
                  e.currentTarget.style.transform = 'translateY(0)';
                  e.currentTarget.style.boxShadow = 'var(--shadow-light)';
                }}
              >
                <Icon name="import" size={14} style={{ marginRight: '4px' }} />
                导入
              </button>
            </div>

            {/* 第二行：搜索、筛选器和设置 */}
            <div className="flex flex-wrap items-center gap-2">
              <div className="search-input-wrapper">
                <Icon name="search" size={14} className="search-input-icon" />
                <input
                  type="text"
                  className="search-input"
                  placeholder="搜索邮箱、用户名或标签..."
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  aria-label="搜索账户"
                />
                {searchQuery && (
                  <button
                    type="button"
                    className="search-input-clear"
                    onClick={() => setSearchQuery('')}
                    aria-label="清除搜索"
                  >
                    <Icon name="close" size={12} />
                  </button>
                )}
              </div>
              <Dropdown
                options={subscriptionFilterOptions}
                value={subscriptionFilter}
                onChange={(value) => setSubscriptionFilter(value)}
              />
              {tagFilterOptions.length > 1 && (
                <Dropdown
                  options={tagFilterOptions}
                  value={tagFilter}
                  onChange={(value) => setTagFilter(value)}
                />
              )}

              {/* 排序控制 */}
              <div style={{ 
                height: '20px', 
                width: '1px', 
                backgroundColor: 'var(--border-primary)',
                margin: '0 4px'
              }} />
              <div className="flex items-center gap-1">
                <Dropdown
                  options={sortOptions}
                  value={sortField}
                  onChange={(value) => updateSort(value, sortOrder)}
                />
                <button
                  type="button"
                  onClick={() => updateSort(sortField, sortOrder === 'asc' ? 'desc' : 'asc')}
                  disabled={sortField === 'none'}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'center',
                    width: '32px',
                    height: '32px',
                    padding: '0',
                    fontSize: '13px',
                    backgroundColor: 'var(--bg-primary)',
                    color: sortField === 'none' ? 'var(--text-tertiary)' : 'var(--text-primary)',
                    border: '1px solid var(--border-primary)',
                    borderRadius: 'var(--border-radius)',
                    cursor: sortField === 'none' ? 'not-allowed' : 'pointer',
                    transition: 'all var(--transition-duration) ease',
                    opacity: sortField === 'none' ? 0.5 : 1,
                  }}
                  title={sortOrder === 'asc' ? '升序 (点击切换为降序)' : '降序 (点击切换为升序)'}
                >
                  <Icon 
                    name={sortOrder === 'asc' ? 'arrow-up' : 'arrow-down'} 
                    size={14} 
                  />
                </button>
              </div>

              <div className="flex items-center gap-2">
                <label
                  className="text-xs font-medium"
                  style={{ color: 'var(--text-secondary)' }}
                >
                  并发:
                </label>
                <input
                  type="number"
                  min="1"
                  value={concurrentLimit}
                  onChange={(e) => {
                    const value = parseInt(e.target.value, 10);
                    if (value >= 1) {
                      setConcurrentLimit(value);
                      safeStorage.set('refresh_concurrent_limit', value);
                    }
                  }}
                  style={{
                    width: '50px',
                    padding: '6px 8px',
                    fontSize: '13px',
                    backgroundColor: 'var(--bg-primary)',
                    color: 'var(--text-primary)',
                    border: '1px solid var(--border-primary)',
                    borderRadius: 'var(--border-radius)',
                    textAlign: 'center',
                    transition: 'all var(--transition-duration) ease',
                  }}
                  onFocus={(e) => {
                    e.currentTarget.style.outline = 'none';
                    e.currentTarget.style.borderColor = 'var(--primary-color)';
                    e.currentTarget.style.boxShadow = '0 0 0 3px rgba(74, 137, 220, 0.1)';
                  }}
                  onBlur={(e) => {
                    e.currentTarget.style.borderColor = 'var(--border-primary)';
                    e.currentTarget.style.boxShadow = 'none';
                  }}
                />
              </div>
            </div>

            {/* 刷新进度条 */}
            {refreshProgress.isRefreshing && (
              <div className="mt-3">
                <div className="flex items-center justify-between mb-2">
                  <span className="text-sm font-medium" style={{ color: 'var(--text-secondary)' }}>
                    刷新进度: {refreshProgress.current} / {refreshProgress.total}
                  </span>
                  <span className="text-sm font-medium" style={{ color: 'var(--primary-color)' }}>
                    {Math.round((refreshProgress.current / refreshProgress.total) * 100)}%
                  </span>
                </div>
                <div
                  className="w-full h-2 overflow-hidden"
                  style={{ 
                    backgroundColor: 'var(--bg-secondary)',
                    borderRadius: '12px'
                  }}
                >
                  <div
                    className="h-2 transition-all duration-300"
                    style={{
                      width: `${(refreshProgress.current / refreshProgress.total) * 100}%`,
                      backgroundColor: 'var(--primary-color)',
                      borderRadius: '12px'
                    }}
                  />
                </div>
              </div>
            )}
          </div>
        </div>



        {/* 账户列表 */}
        <div className="px-4 py-4" style={{ overflow: 'visible' }}>
          <div className="flex items-center justify-between mb-3">
            <div className="flex items-center space-x-2">
              <input
                type="checkbox"
                checked={isAllSelected}
                onChange={toggleSelectAll}
                style={{
                  width: '16px',
                  height: '16px',
                  accentColor: 'var(--primary-color)',
                  cursor: 'pointer',
                }}
              />
              <h4 className="font-medium text-md" style={{ color: 'var(--text-primary)' }}>
                账户列表 {selectedAccounts.size > 0 && `(已选 ${selectedAccounts.size})`}
              </h4>
            </div>
            {accountData?.accounts && accountData.accounts.length > 0 && (
              <span className="text-sm" style={{ color: 'var(--text-secondary)' }}>
                {subscriptionFilter === "all" && !searchQuery
                  ? `共 ${accountData.accounts.length} 个账户`
                  : `显示 ${searchFilteredAccounts.length} / ${accountData.accounts.length} 个账户`
                }
                {searchQuery && ` (搜索结果)`}
              </span>
            )}
          </div>

          {/* 使用虚拟滚动或常规渲染 */}
          {accountData?.accounts && accountData.accounts.length > 0 ? (
            shouldUseVirtualScroll ? (
              <VirtualizedAccountList
                accounts={searchFilteredAccounts}
                renderItem={renderAccountCard}
                height={600}
                itemSize={60} // ✅ 紧凑布局：减小行高
                style={{ borderRadius: 'var(--border-radius)' }}
                overscanCount={5}
              />
            ) : (
              <div className="space-y-2" style={{ overflow: 'visible' }}>
                {searchFilteredAccounts.map((account, index) => renderAccountCard(account, index))}
              </div>
            )
          ) : (
            <div className="py-12 text-center" style={{ color: 'var(--text-secondary)' }}>
              <p>暂无账户，点击"添加账户"开始</p>
            </div>
          )}
        </div>
      </div>

      {/* Toast 提示 */}
      {toast && (
        <Toast
          message={toast.message}
          type={toast.type}
          onClose={() => setToast(null)}
        />
      )}

      {/* 确认对话框 */}
      {confirmDialog.show && (
        <ConfirmDialog
          isOpen={confirmDialog.show}
          title={confirmDialog.title}
          message={confirmDialog.message}
          onConfirm={confirmDialog.onConfirm}
          onCancel={() => setConfirmDialog({ ...confirmDialog, show: false })}
          checkboxLabel={confirmDialog.checkboxLabel}
          checkboxDefaultChecked={confirmDialog.checkboxDefaultChecked}
          type={confirmDialog.type}
          confirmText={confirmDialog.confirmText}
        />
      )}

      {/* 切换账号选项弹窗 */}
      {switchModal.show && switchModal.account && (
        <div className="fixed inset-0 z-50 flex items-center justify-center">
          <div className="fixed inset-0" style={{ backgroundColor: 'rgba(0,0,0,0.5)' }} onClick={() => setSwitchModal(prev => ({ ...prev, show: false }))} />
          <div className="relative z-10 w-[400px] rounded-lg p-6" style={{ backgroundColor: 'var(--bg-primary)', boxShadow: 'var(--shadow-heavy)' }}>
            <h3 className="text-lg font-semibold mb-4" style={{ color: 'var(--text-primary)' }}>切换账号</h3>
            <p className="text-sm mb-4" style={{ color: 'var(--text-secondary)' }}>
              确定要使用账号 <strong>{switchModal.account.email}</strong> 吗？
            </p>

            {/* Cursor 操作选项 */}
            <div className="mb-3">
              <span className="text-xs mb-2 block" style={{ color: 'var(--text-secondary)' }}>Cursor 操作</span>
              <div className="space-y-1">
                <label className="flex items-center gap-2 cursor-pointer">
                  <input
                    type="radio"
                    name="cursorAction"
                    checked={switchModal.cursorAction === "restart"}
                    onChange={() => setSwitchModal(prev => ({ ...prev, cursorAction: "restart" }))}
                    style={{ accentColor: 'var(--primary-color)' }}
                  />
                  <span className="text-sm" style={{ color: 'var(--text-primary)' }}>关闭并重启 Cursor</span>
                </label>
                <label className="flex items-center gap-2 cursor-pointer">
                  <input
                    type="radio"
                    name="cursorAction"
                    checked={switchModal.cursorAction === "close_only"}
                    onChange={() => setSwitchModal(prev => ({ ...prev, cursorAction: "close_only" }))}
                    style={{ accentColor: 'var(--primary-color)' }}
                  />
                  <span className="text-sm" style={{ color: 'var(--text-primary)' }}>仅关闭 Cursor（不自动启动）</span>
                </label>
                <label className="flex items-center gap-2 cursor-pointer">
                  <input
                    type="radio"
                    name="cursorAction"
                    checked={switchModal.cursorAction === "none"}
                    onChange={() => setSwitchModal(prev => ({ ...prev, cursorAction: "none" }))}
                    style={{ accentColor: 'var(--primary-color)' }}
                  />
                  <span className="text-sm" style={{ color: 'var(--text-primary)' }}>不操作 Cursor（稍后手动重启）</span>
                </label>
              </div>
            </div>

            {/* 重置机器码勾选 */}
            <label className="flex items-center gap-2 mb-3 cursor-pointer">
              <input
                type="checkbox"
                checked={switchModal.resetMachineId}
                onChange={(e) => setSwitchModal(prev => ({ ...prev, resetMachineId: e.target.checked }))}
                style={{ accentColor: 'var(--primary-color)' }}
              />
              <span className="text-sm font-medium" style={{ color: 'var(--text-primary)' }}>重置机器码（推荐）</span>
            </label>

            {/* 机器码选项 */}
            {switchModal.resetMachineId && (
              <div className="ml-6 space-y-2 mb-4">
                <label className="flex items-center gap-2 cursor-pointer">
                  <input
                    type="radio"
                    name="machineIdOption"
                    checked={switchModal.machineIdOption === "bound"}
                    onChange={() => setSwitchModal(prev => ({ ...prev, machineIdOption: "bound" }))}
                    style={{ accentColor: 'var(--primary-color)' }}
                  />
                  <span className="text-sm" style={{ color: switchModal.account.machine_ids ? 'var(--text-primary)' : 'var(--text-tertiary)' }}>
                    使用该账号已绑定的机器码
                    {!switchModal.account.machine_ids && <span className="text-xs ml-1">(无绑定)</span>}
                  </span>
                </label>
                <label className="flex items-center gap-2 cursor-pointer">
                  <input
                    type="radio"
                    name="machineIdOption"
                    checked={switchModal.machineIdOption === "new"}
                    onChange={() => setSwitchModal(prev => ({ ...prev, machineIdOption: "new" }))}
                    style={{ accentColor: 'var(--primary-color)' }}
                  />
                  <span className="text-sm" style={{ color: 'var(--text-primary)' }}>随机新的机器码并绑定到账号</span>
                </label>
              </div>
            )}

            {/* 按钮 */}
            <div className="flex justify-end gap-3 mt-4 pt-4" style={{ borderTop: '1px solid var(--border-primary)' }}>
              <button
                onClick={() => setSwitchModal(prev => ({ ...prev, show: false }))}
                className="px-4 py-2 text-sm rounded"
                style={{ backgroundColor: 'var(--bg-secondary)', color: 'var(--text-primary)', border: '1px solid var(--border-primary)' }}
              >
                取消
              </button>
              <button
                onClick={handleSwitchConfirm}
                className="px-4 py-2 text-sm rounded font-medium"
                style={{ backgroundColor: 'var(--primary-color)', color: 'white', border: 'none' }}
              >
                确定
              </button>
            </div>
          </div>
        </div>
      )}

      {/* 用量统计 Modal */}
      {usageModalOpen && selectedAccountUsage && (
        <div className="fixed inset-0 z-50 overflow-y-auto">
          <div className="flex items-center justify-center min-h-screen px-4">
            <div 
              className="fixed inset-0 transition-opacity" 
              style={{ backgroundColor: 'rgba(0, 0, 0, 0.5)' }}
              onClick={() => setUsageModalOpen(false)} 
            />
            <div 
              className="relative z-10 w-[95%] max-w-[750px] max-h-[90vh] overflow-hidden"
              style={{
                backgroundColor: 'var(--bg-primary)',
                boxShadow: 'var(--shadow-heavy)',
                backdropFilter: 'blur(var(--backdrop-blur))',
                WebkitBackdropFilter: 'blur(var(--backdrop-blur))',
                borderRadius: 'var(--border-radius-large)',
              }}
            >
              <div className="p-6">
                <div className="flex items-center justify-between mb-4">
                  <h3 className="text-lg font-medium" style={{ color: 'var(--text-primary)' }}>
                    用量统计 - {selectedAccountUsage.account.email}
                  </h3>
                  <button
                    onClick={() => setUsageModalOpen(false)}
                    style={{
                      color: 'var(--text-secondary)',
                      transition: 'color var(--transition-duration) ease',
                    }}
                    onMouseEnter={(e) => {
                      e.currentTarget.style.color = 'var(--text-primary)';
                    }}
                    onMouseLeave={(e) => {
                      e.currentTarget.style.color = 'var(--text-secondary)';
                    }}
                  >
                    <Icon name="close" size={20} />
                  </button>
                </div>

                {/* UsageDisplay 组件 */}
                <div className="overflow-y-auto max-h-[calc(90vh-120px)]">
                  <UsageDisplay
                    token={selectedAccountUsage.account.token}
                    email={selectedAccountUsage.account.email}
                    className="mt-4"
                    hideHeader={true}
                    onUsageCostUpdated={(totalCostCents) => {
                      updateAccountUsageCost(selectedAccountUsage.account.email, totalCostCents);
                    }}
                  />
                </div>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* 添加账户 Modal */}
      <AddAccountForm
        isOpen={showAddForm}
        onSuccess={handleAddSuccess}
        onCancel={() => setShowAddForm(false)}
        onToast={(message, type) => setToast({ message, type })}
      />

      {/* 编辑账户 Modal */}
      <EditAccountForm
        isOpen={showEditForm}
        account={editingAccount}
        onSuccess={handleEditSuccess}
        onCancel={() => {
          setShowEditForm(false);
          setEditingAccount(null);
        }}
        onToast={(message, type) => setToast({ message, type })}
      />
    </div>
  );
};


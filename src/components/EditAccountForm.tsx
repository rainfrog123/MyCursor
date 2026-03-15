import { useState, useEffect, memo, useCallback } from "react";
import { AccountService } from "../services/accountService";
import type { AccountInfo, MachineIds } from "../types/account";
import { FormField, TextInput, TextareaInput } from "./form/FormField";
import { TagSelector } from "./TagSelector";
import Modal from "./Modal";

interface EditAccountFormProps {
  isOpen: boolean;
  account: AccountInfo | null;
  onSuccess: () => void;
  onCancel: () => void;
  onToast: (message: string, type: "success" | "error") => void;
}

export const EditAccountForm = memo(({ isOpen, account, onSuccess, onCancel, onToast }: EditAccountFormProps) => {
  // ============================================================
  // 📦 状态管理
  // ============================================================
  const [editEmail, setEditEmail] = useState("");
  const [editToken, setEditToken] = useState("");
  const [editRefreshToken, setEditRefreshToken] = useState("");
  const [editWorkosSessionToken, setEditWorkosSessionToken] = useState("");
  const [editStripeUrl, setEditStripeUrl] = useState("");
  const [editUsername, setEditUsername] = useState("");
  const [editTags, setEditTags] = useState<string[]>([]);
  const [showMachineIds, setShowMachineIds] = useState(false);
  const [editMachineIds, setEditMachineIds] = useState<Partial<MachineIds>>({});
  const [machineIdsJson, setMachineIdsJson] = useState("");
  const [machineIdsParseError, setMachineIdsParseError] = useState("");

  useEffect(() => {
    if (account) {
      setEditEmail(account.email || "");
      setEditToken(account.token || "");
      setEditRefreshToken(account.refresh_token || "");
      setEditWorkosSessionToken(account.workos_cursor_session_token || "");
      setEditStripeUrl(account.stripe_url || "");
      setEditUsername(account.username || "");
      setEditTags(account.tags || []);
      setShowMachineIds(!!account.machine_ids);
      setEditMachineIds(account.machine_ids || {});
      setMachineIdsJson(account.machine_ids ? JSON.stringify(account.machine_ids, null, 2) : "");
      setMachineIdsParseError("");
    }
  }, [account]);

  // ============================================================
  // 🎯 业务逻辑（使用 useCallback 优化）
  // ============================================================
  const REQUIRED_MACHINE_ID_KEYS = [
    "telemetry.devDeviceId", "telemetry.macMachineId",
    "telemetry.machineId", "telemetry.sqmId", "storage.serviceMachineId",
  ] as const;

  const OPTIONAL_MACHINE_ID_KEYS = [
    "system.machineGuid", "system.sqmClientId",
  ] as const;

  const parseMachineIdsFromJson = useCallback((json: string) => {
    if (!json.trim()) {
      setEditMachineIds({});
      setMachineIdsParseError("");
      return;
    }
    try {
      const data = JSON.parse(json);
      const ids: Record<string, string> = {};
      const missing: string[] = [];
      for (const key of REQUIRED_MACHINE_ID_KEYS) {
        if (data[key] && typeof data[key] === "string") {
          ids[key] = data[key];
        } else {
          missing.push(key.split(".").pop() || key);
        }
      }
      for (const key of OPTIONAL_MACHINE_ID_KEYS) {
        if (data[key] && typeof data[key] === "string") {
          ids[key] = data[key];
        }
      }
      if (missing.length > 0) {
        setMachineIdsParseError(`缺少必须字段: ${missing.join(", ")}`);
        setEditMachineIds({});
      } else {
        setEditMachineIds(ids as any);
        setMachineIdsParseError("");
      }
    } catch {
      setMachineIdsParseError("JSON 格式错误，请粘贴完整的 storage.json 内容");
      setEditMachineIds({});
    }
  }, []);

  const handleSave = useCallback(async () => {
    if (!account) return;

    try {
      // 检查邮箱格式
      if (editEmail && !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(editEmail)) {
        onToast("请输入有效的邮箱地址", "error");
        return;
      }

      // 机器码验证：至少包含 5 个必须字段
      if (showMachineIds && machineIdsJson.trim()) {
        const hasAllRequired = REQUIRED_MACHINE_ID_KEYS.every((k) => (editMachineIds as any)[k]);
        if (machineIdsParseError || !hasAllRequired) {
          onToast("机器码绑定失败：请确保包含全部 5 个必须字段", "error");
          return;
        }
      }
      const hasRequired = REQUIRED_MACHINE_ID_KEYS.every((k) => (editMachineIds as any)[k]);
      const machineIdsToSave = showMachineIds && hasRequired ? editMachineIds : undefined;

      const result = await AccountService.editAccount(
        account.email,
        editEmail !== account.email ? editEmail : undefined,
        editToken || undefined,
        editRefreshToken || undefined,
        editWorkosSessionToken || undefined,
        editStripeUrl || undefined,
        editUsername || undefined,
        editTags,
        machineIdsToSave
      );

      if (result.success) {
        onToast("账户信息已更新", "success");
        onSuccess();
      } else {
        onToast(result.message, "error");
      }
    } catch (error) {
      console.error("编辑账户失败:", error);
      onToast("更新账户信息失败", "error");
    }
  }, [
    account,
    editEmail,
    editToken,
    editRefreshToken,
    editWorkosSessionToken,
    editStripeUrl,
    editUsername,
    editTags,
    showMachineIds,
    editMachineIds,
    machineIdsJson,
    machineIdsParseError,
    onToast,
    onSuccess,
  ]);

  if (!account) return null;

  // ============================================================
  // 🎨 渲染
  // ============================================================
  return (
    <Modal
      open={isOpen}
      onClose={onCancel}
      title={`编辑账户 - ${account.email}`}
      size="lg"
      footer={
        <>
          <button
            onClick={onCancel}
            style={{
              padding: '8px 16px',
              fontSize: '14px',
              fontWeight: '500',
              color: 'var(--text-primary)',
              backgroundColor: 'var(--bg-primary)',
              border: '1px solid var(--border-primary)',
              borderRadius: 'var(--border-radius)',
              cursor: 'pointer',
              transition: 'all var(--transition-duration) ease',
            }}
            onMouseEnter={(e) => {
              e.currentTarget.style.backgroundColor = 'var(--bg-secondary)';
            }}
            onMouseLeave={(e) => {
              e.currentTarget.style.backgroundColor = 'var(--bg-primary)';
            }}
          >
            取消
          </button>
          <button
            onClick={handleSave}
            style={{
              padding: '8px 16px',
              fontSize: '14px',
              fontWeight: '500',
              color: 'white',
              backgroundColor: 'var(--primary-color)',
              border: 'none',
              borderRadius: 'var(--border-radius)',
              cursor: 'pointer',
              transition: 'all var(--transition-duration) ease',
            }}
            onMouseEnter={(e) => {
              e.currentTarget.style.transform = 'translateY(-1px)';
              e.currentTarget.style.boxShadow = 'var(--shadow-medium)';
            }}
            onMouseLeave={(e) => {
              e.currentTarget.style.transform = 'translateY(0)';
              e.currentTarget.style.boxShadow = 'none';
            }}
          >
            保存更改
          </button>
        </>
      }
    >
      <div className="space-y-4">
        {/* 邮箱地址 */}
        <FormField label="邮箱地址">
          <TextInput
            type="email"
            value={editEmail}
            onChange={setEditEmail}
            placeholder="设置账户邮箱地址"
          />
        </FormField>

        {/* 用户名 */}
        <FormField label="用户名">
          <TextInput
            type="text"
            value={editUsername}
            onChange={setEditUsername}
            placeholder="设置用户名备注"
          />
        </FormField>

        {/* Workos Session Token */}
        <FormField
          label="Workos Session Token"
          description="用于主页登录、取消订阅、绑卡等功能"
        >
          <TextareaInput
            value={editWorkosSessionToken}
            onChange={setEditWorkosSessionToken}
            placeholder="更新 Workos Session Token"
            rows={3}
          />
        </FormField>

        {/* Stripe URL */}
        <FormField label="Stripe URL" description="Stripe 支付链接（可选）">
          <TextInput
            type="text"
            value={editStripeUrl}
            onChange={setEditStripeUrl}
            placeholder="https://checkout.stripe.com/..."
          />
        </FormField>

        {/* Access Token */}
        <FormField label="Access Token">
          <TextareaInput
            value={editToken}
            onChange={setEditToken}
            placeholder="更新 Access Token"
            rows={3}
          />
        </FormField>

        {/* Refresh Token */}
        <FormField label="Refresh Token" description="用于自动刷新 Access Token（可选）">
          <TextareaInput
            value={editRefreshToken}
            onChange={setEditRefreshToken}
            placeholder="更新 Refresh Token"
            rows={3}
          />
        </FormField>

        {/* 机器码（可选） */}
        <div>
          <button
            type="button"
            onClick={() => setShowMachineIds(!showMachineIds)}
            className="flex items-center gap-1 text-sm font-medium mb-2"
            style={{ color: 'var(--primary-color)', background: 'none', border: 'none', cursor: 'pointer', padding: 0 }}
          >
            <span style={{ transform: showMachineIds ? 'rotate(90deg)' : 'rotate(0)', transition: '0.2s', display: 'inline-block' }}>&#9654;</span>
            绑定机器码（可选）
          </button>
          {showMachineIds && (
            <div className="p-3 rounded" style={{ backgroundColor: 'var(--bg-secondary)', border: '1px solid var(--border-primary)' }}>
              <p className="text-xs mb-2" style={{ color: 'var(--text-tertiary)' }}>
                粘贴 storage.json 内容，自动提取 5 个必须字段。
                如需绑定注册表 ID（machineGuid、sqmClientId），请手动添加到 JSON 中。
                <br />路径：%APPDATA%\Cursor\User\globalStorage\storage.json
              </p>
              <TextareaInput
                value={machineIdsJson}
                onChange={(val) => {
                  setMachineIdsJson(val);
                  parseMachineIdsFromJson(val);
                }}
                placeholder='粘贴 storage.json 内容（JSON 格式）'
                rows={4}
              />
              {machineIdsParseError && (
                <p className="text-xs mt-1" style={{ color: '#ef4444' }}>{machineIdsParseError}</p>
              )}
              {!machineIdsParseError && Object.keys(editMachineIds).length >= 5 && (
                <div className="mt-2 text-xs space-y-0.5" style={{ color: '#10b981' }}>
                  {Object.entries(editMachineIds).map(([k, v]) => (
                    <p key={k} className="font-mono truncate">{k.split('.').pop()}: {String(v).slice(0, 30)}...</p>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>

        {/* 标签管理 */}
        <FormField label="标签" description="选择预设标签或输入新标签回车创建">
          <TagSelector
            selectedTags={editTags}
            onChange={setEditTags}
          />
        </FormField>
      </div>
    </Modal>
  );
});

EditAccountForm.displayName = "EditAccountForm";


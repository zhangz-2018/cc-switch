import { CSS } from "@dnd-kit/utilities";
import { DndContext, closestCenter } from "@dnd-kit/core";
import {
  SortableContext,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { AnimatePresence, motion } from "framer-motion";
import { RefreshCw, Search, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import type { Provider } from "@/types";
import type { AppId } from "@/lib/api";
import { providersApi } from "@/lib/api/providers";
import { codexApi } from "@/lib/api/codex";
import { usageApi } from "@/lib/api/usage";
import { useDragSort } from "@/hooks/useDragSort";
// import { useStreamCheck } from "@/hooks/useStreamCheck"; // 测试功能已隐藏
import { ProviderCard } from "@/components/providers/ProviderCard";
import { ProviderEmptyState } from "@/components/providers/ProviderEmptyState";
import {
  useAutoFailoverEnabled,
  useFailoverQueue,
  useAddToFailoverQueue,
  useRemoveFromFailoverQueue,
} from "@/lib/query/failover";
import { useCallback } from "react";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";

interface ProviderListProps {
  providers: Record<string, Provider>;
  currentProviderId: string;
  appId: AppId;
  onSwitch: (provider: Provider) => void;
  onEdit: (provider: Provider) => void;
  onDelete: (provider: Provider) => void;
  /** OpenCode: remove from live config (not delete from database) */
  onRemoveFromConfig?: (provider: Provider) => void;
  onDuplicate: (provider: Provider) => void;
  onConfigureUsage?: (provider: Provider) => void;
  onOpenWebsite: (url: string) => void;
  onOpenTerminal?: (provider: Provider) => void;
  onCreate?: () => void;
  isLoading?: boolean;
  isProxyRunning?: boolean; // 代理服务运行状态
  isProxyTakeover?: boolean; // 代理接管模式（Live配置已被接管）
  activeProviderId?: string; // 代理当前实际使用的供应商 ID（用于故障转移模式下标注绿色边框）
}

export function ProviderList({
  providers,
  currentProviderId,
  appId,
  onSwitch,
  onEdit,
  onDelete,
  onRemoveFromConfig,
  onDuplicate,
  onConfigureUsage,
  onOpenWebsite,
  onOpenTerminal,
  onCreate,
  isLoading = false,
  isProxyRunning = false,
  isProxyTakeover = false,
  activeProviderId,
}: ProviderListProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const { sortedProviders, sensors, handleDragEnd } = useDragSort(
    providers,
    appId,
  );
  const [isRefreshingCodexQuota, setIsRefreshingCodexQuota] = useState(false);
  const [isRefreshingGeminiUsage, setIsRefreshingGeminiUsage] = useState(false);

  // OpenCode: 查询 live 配置中的供应商 ID 列表，用于判断 isInConfig
  const { data: opencodeLiveIds } = useQuery({
    queryKey: ["opencodeLiveProviderIds"],
    queryFn: () => providersApi.getOpenCodeLiveProviderIds(),
    enabled: appId === "opencode",
  });

  // OpenCode: 判断供应商是否已添加到 opencode.json
  const isProviderInConfig = useCallback(
    (providerId: string): boolean => {
      if (appId !== "opencode") return true; // 非 OpenCode 应用始终返回 true
      return opencodeLiveIds?.includes(providerId) ?? false;
    },
    [appId, opencodeLiveIds],
  );

  // 流式健康检查 - 功能已隐藏
  // const { checkProvider, isChecking } = useStreamCheck(appId);

  // 故障转移相关
  const { data: isAutoFailoverEnabled } = useAutoFailoverEnabled(appId);
  const { data: failoverQueue } = useFailoverQueue(appId);
  const addToQueue = useAddToFailoverQueue();
  const removeFromQueue = useRemoveFromFailoverQueue();

  // 联动状态：只有当前应用开启代理接管且故障转移开启时才启用故障转移模式
  const isFailoverModeActive =
    isProxyTakeover === true && isAutoFailoverEnabled === true;

  // 计算供应商在故障转移队列中的优先级（基于 sortIndex 排序）
  const getFailoverPriority = useCallback(
    (providerId: string): number | undefined => {
      if (!isFailoverModeActive || !failoverQueue) return undefined;
      const index = failoverQueue.findIndex(
        (item) => item.providerId === providerId,
      );
      return index >= 0 ? index + 1 : undefined;
    },
    [isFailoverModeActive, failoverQueue],
  );

  // 判断供应商是否在故障转移队列中
  const isInFailoverQueue = useCallback(
    (providerId: string): boolean => {
      if (!isFailoverModeActive || !failoverQueue) return false;
      return failoverQueue.some((item) => item.providerId === providerId);
    },
    [isFailoverModeActive, failoverQueue],
  );

  // 切换供应商的故障转移队列状态
  const handleToggleFailover = useCallback(
    (providerId: string, enabled: boolean) => {
      if (enabled) {
        addToQueue.mutate({ appType: appId, providerId });
      } else {
        removeFromQueue.mutate({ appType: appId, providerId });
      }
    },
    [appId, addToQueue, removeFromQueue],
  );

  // handleTest 功能已隐藏 - 供应商请求格式复杂难以统一测试
  // const handleTest = (provider: Provider) => {
  //   checkProvider(provider.id, provider.name);
  // };

  const [searchTerm, setSearchTerm] = useState("");
  const [isSearchOpen, setIsSearchOpen] = useState(false);
  const searchInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      const key = event.key.toLowerCase();
      if ((event.metaKey || event.ctrlKey) && key === "f") {
        event.preventDefault();
        setIsSearchOpen(true);
        return;
      }

      if (key === "escape") {
        setIsSearchOpen(false);
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  useEffect(() => {
    if (isSearchOpen) {
      const frame = requestAnimationFrame(() => {
        searchInputRef.current?.focus();
        searchInputRef.current?.select();
      });
      return () => cancelAnimationFrame(frame);
    }
  }, [isSearchOpen]);

  const filteredProviders = useMemo(() => {
    const keyword = searchTerm.trim().toLowerCase();
    if (!keyword) return sortedProviders;
    return sortedProviders.filter((provider) => {
      const fields = [provider.name, provider.notes, provider.websiteUrl];
      return fields.some((field) =>
        field?.toString().toLowerCase().includes(keyword),
      );
    });
  }, [searchTerm, sortedProviders]);

  const codexOfficialProviders = useMemo(() => {
    if (appId !== "codex") return [];
    return sortedProviders.filter((provider) => provider.category === "official");
  }, [appId, sortedProviders]);

  const geminiUsageProviders = useMemo(() => {
    if (appId !== "gemini") return [];
    return sortedProviders.filter(
      (provider) =>
        provider.meta?.usage_script?.enabled === true ||
        provider.meta?.partnerPromotionKey?.toLowerCase() === "antigravity",
    );
  }, [appId, sortedProviders]);

  const handleRefreshCodexQuota = useCallback(async () => {
    if (appId !== "codex" || codexOfficialProviders.length === 0) {
      return;
    }

    setIsRefreshingCodexQuota(true);
    try {
      const results = await Promise.allSettled(
        codexOfficialProviders.map((provider) =>
          queryClient.fetchQuery({
            queryKey: ["codex-quota", provider.id],
            queryFn: async () => codexApi.getQuota(provider.id),
            staleTime: 0,
          }),
        ),
      );

      const successCount = results.filter(
        (result) => result.status === "fulfilled",
      ).length;
      const failedCount = results.length - successCount;

      if (failedCount === 0) {
        toast.success(
          t("provider.codexQuotaRefreshAllSuccess", {
            defaultValue: `已刷新 ${successCount} 个官方供应商余量`,
          }),
        );
      } else if (successCount === 0) {
        toast.error(
          t("provider.codexQuotaRefreshAllFailed", {
            defaultValue: "余量刷新失败，请稍后重试",
          }),
        );
      } else {
        toast.warning(
          t("provider.codexQuotaRefreshAllPartial", {
            defaultValue: `已刷新 ${successCount} 个，${failedCount} 个失败`,
          }),
        );
      }
    } finally {
      setIsRefreshingCodexQuota(false);
    }
  }, [appId, codexOfficialProviders, queryClient, t]);

  const handleRefreshGeminiUsage = useCallback(async () => {
    if (appId !== "gemini" || geminiUsageProviders.length === 0) {
      return;
    }

    setIsRefreshingGeminiUsage(true);
    try {
      const results = await Promise.allSettled(
        geminiUsageProviders.map((provider) =>
          queryClient.fetchQuery({
            queryKey: ["usage", provider.id, appId],
            queryFn: async () => usageApi.query(provider.id, appId),
            staleTime: 0,
          }),
        ),
      );

      const successCount = results.filter(
        (result) => result.status === "fulfilled",
      ).length;
      const failedCount = results.length - successCount;

      if (failedCount === 0) {
        toast.success(
          t("provider.geminiUsageRefreshAllSuccess", {
            defaultValue: `已刷新 ${successCount} 个 Gemini 供应商余量`,
          }),
        );
      } else if (successCount === 0) {
        toast.error(
          t("provider.geminiUsageRefreshAllFailed", {
            defaultValue: "Gemini 余量刷新失败，请稍后重试",
          }),
        );
      } else {
        toast.warning(
          t("provider.geminiUsageRefreshAllPartial", {
            defaultValue: `已刷新 ${successCount} 个，${failedCount} 个失败`,
          }),
        );
      }
    } finally {
      setIsRefreshingGeminiUsage(false);
    }
  }, [appId, geminiUsageProviders, queryClient, t]);

  if (isLoading) {
    return (
      <div className="space-y-3">
        {[0, 1, 2].map((index) => (
          <div
            key={index}
            className="w-full border border-dashed rounded-lg h-28 border-muted-foreground/40 bg-muted/40"
          />
        ))}
      </div>
    );
  }

  if (sortedProviders.length === 0) {
    return <ProviderEmptyState onCreate={onCreate} />;
  }

  const renderProviderList = () => (
    <DndContext
      sensors={sensors}
      collisionDetection={closestCenter}
      onDragEnd={handleDragEnd}
    >
      <SortableContext
        items={filteredProviders.map((provider) => provider.id)}
        strategy={verticalListSortingStrategy}
      >
        <div className="space-y-3">
          {filteredProviders.map((provider) => (
            <SortableProviderCard
              key={provider.id}
              provider={provider}
              isCurrent={provider.id === currentProviderId}
              appId={appId}
              isInConfig={isProviderInConfig(provider.id)}
              onSwitch={onSwitch}
              onEdit={onEdit}
              onDelete={onDelete}
              onRemoveFromConfig={onRemoveFromConfig}
              onDuplicate={onDuplicate}
              onConfigureUsage={onConfigureUsage}
              onOpenWebsite={onOpenWebsite}
              onOpenTerminal={onOpenTerminal}
              // onTest 功能已隐藏 - 供应商请求格式复杂难以统一测试
              // onTest={appId !== "opencode" ? handleTest : undefined}
              isTesting={false} // isChecking(provider.id) - 测试功能已隐藏
              isProxyRunning={isProxyRunning}
              isProxyTakeover={isProxyTakeover}
              // 故障转移相关：联动状态
              isAutoFailoverEnabled={isFailoverModeActive}
              failoverPriority={getFailoverPriority(provider.id)}
              isInFailoverQueue={isInFailoverQueue(provider.id)}
              onToggleFailover={(enabled) =>
                handleToggleFailover(provider.id, enabled)
              }
              activeProviderId={activeProviderId}
            />
          ))}
        </div>
      </SortableContext>
    </DndContext>
  );

  return (
    <div className="mt-4 space-y-4">
      {((appId === "codex" && codexOfficialProviders.length > 0) ||
        (appId === "gemini" && geminiUsageProviders.length > 0)) && (
        <div className="flex justify-end">
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={appId === "codex" ? handleRefreshCodexQuota : handleRefreshGeminiUsage}
            disabled={isRefreshingCodexQuota || isRefreshingGeminiUsage}
            className="gap-2"
          >
            <RefreshCw
              className={
                isRefreshingCodexQuota || isRefreshingGeminiUsage
                  ? "h-4 w-4 animate-spin"
                  : "h-4 w-4"
              }
            />
            {isRefreshingCodexQuota || isRefreshingGeminiUsage
              ? t("provider.codexQuotaRefreshing", {
                  defaultValue: "刷新中...",
                })
              : t("provider.codexQuotaRefresh", {
                  defaultValue: "刷新余量",
                })}
          </Button>
        </div>
      )}

      <AnimatePresence>
        {isSearchOpen && (
          <motion.div
            key="provider-search"
            initial={{ opacity: 0, y: -8, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -8, scale: 0.98 }}
            transition={{ duration: 0.18, ease: "easeOut" }}
            className="fixed left-1/2 top-[6.5rem] z-40 w-[min(90vw,26rem)] -translate-x-1/2 sm:right-6 sm:left-auto sm:translate-x-0"
          >
            <div className="p-4 space-y-3 border shadow-md rounded-2xl border-white/10 bg-background/95 shadow-black/20 backdrop-blur-md">
              <div className="relative flex items-center gap-2">
                <Search className="absolute w-4 h-4 -translate-y-1/2 pointer-events-none left-3 top-1/2 text-muted-foreground" />
                <Input
                  ref={searchInputRef}
                  value={searchTerm}
                  onChange={(event) => setSearchTerm(event.target.value)}
                  placeholder={t("provider.searchPlaceholder", {
                    defaultValue: "Search name, notes, or URL...",
                  })}
                  aria-label={t("provider.searchAriaLabel", {
                    defaultValue: "Search providers",
                  })}
                  className="pr-16 pl-9"
                />
                {searchTerm && (
                  <Button
                    variant="ghost"
                    size="sm"
                    className="absolute text-xs -translate-y-1/2 right-11 top-1/2"
                    onClick={() => setSearchTerm("")}
                  >
                    {t("common.clear", { defaultValue: "Clear" })}
                  </Button>
                )}
                <Button
                  variant="ghost"
                  size="icon"
                  className="ml-auto"
                  onClick={() => setIsSearchOpen(false)}
                  aria-label={t("provider.searchCloseAriaLabel", {
                    defaultValue: "Close provider search",
                  })}
                >
                  <X className="w-4 h-4" />
                </Button>
              </div>
              <div className="flex flex-wrap items-center justify-between gap-2 text-[11px] text-muted-foreground">
                <span>
                  {t("provider.searchScopeHint", {
                    defaultValue: "Matches provider name, notes, and URL.",
                  })}
                </span>
                <span>
                  {t("provider.searchCloseHint", {
                    defaultValue: "Press Esc to close",
                  })}
                </span>
              </div>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {filteredProviders.length === 0 ? (
        <div className="px-6 py-8 text-sm text-center border border-dashed rounded-lg border-border text-muted-foreground">
          {t("provider.noSearchResults", {
            defaultValue: "No providers match your search.",
          })}
        </div>
      ) : (
        renderProviderList()
      )}
    </div>
  );
}

interface SortableProviderCardProps {
  provider: Provider;
  isCurrent: boolean;
  appId: AppId;
  isInConfig: boolean;
  onSwitch: (provider: Provider) => void;
  onEdit: (provider: Provider) => void;
  onDelete: (provider: Provider) => void;
  /** OpenCode: remove from live config (not delete from database) */
  onRemoveFromConfig?: (provider: Provider) => void;
  onDuplicate: (provider: Provider) => void;
  onConfigureUsage?: (provider: Provider) => void;
  onOpenWebsite: (url: string) => void;
  onOpenTerminal?: (provider: Provider) => void;
  onTest?: (provider: Provider) => void;
  isTesting: boolean;
  isProxyRunning: boolean;
  isProxyTakeover: boolean;
  // 故障转移相关
  isAutoFailoverEnabled: boolean;
  failoverPriority?: number;
  isInFailoverQueue: boolean;
  onToggleFailover: (enabled: boolean) => void;
  activeProviderId?: string;
}

function SortableProviderCard({
  provider,
  isCurrent,
  appId,
  isInConfig,
  onSwitch,
  onEdit,
  onDelete,
  onRemoveFromConfig,
  onDuplicate,
  onConfigureUsage,
  onOpenWebsite,
  onOpenTerminal,
  onTest,
  isTesting,
  isProxyRunning,
  isProxyTakeover,
  isAutoFailoverEnabled,
  failoverPriority,
  isInFailoverQueue,
  onToggleFailover,
  activeProviderId,
}: SortableProviderCardProps) {
  const {
    setNodeRef,
    attributes,
    listeners,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: provider.id });

  const style: CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
  };

  return (
    <div ref={setNodeRef} style={style}>
      <ProviderCard
        provider={provider}
        isCurrent={isCurrent}
        appId={appId}
        isInConfig={isInConfig}
        onSwitch={onSwitch}
        onEdit={onEdit}
        onDelete={onDelete}
        onRemoveFromConfig={onRemoveFromConfig}
        onDuplicate={onDuplicate}
        onConfigureUsage={
          onConfigureUsage ? (item) => onConfigureUsage(item) : () => undefined
        }
        onOpenWebsite={onOpenWebsite}
        onOpenTerminal={onOpenTerminal}
        onTest={onTest}
        isTesting={isTesting}
        isProxyRunning={isProxyRunning}
        isProxyTakeover={isProxyTakeover}
        dragHandleProps={{
          attributes,
          listeners,
          isDragging,
        }}
        // 故障转移相关
        isAutoFailoverEnabled={isAutoFailoverEnabled}
        failoverPriority={failoverPriority}
        isInFailoverQueue={isInFailoverQueue}
        onToggleFailover={onToggleFailover}
        activeProviderId={activeProviderId}
      />
    </div>
  );
}

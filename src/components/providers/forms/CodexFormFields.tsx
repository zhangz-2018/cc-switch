import { useTranslation } from "react-i18next";
import EndpointSpeedTest from "./EndpointSpeedTest";
import { ApiKeySection, EndpointField } from "./shared";
import type { ProviderCategory } from "@/types";

interface EndpointCandidate {
  url: string;
}

interface CodexFormFieldsProps {
  providerId?: string;
  // API Key
  codexApiKey: string;
  onApiKeyChange: (key: string) => void;
  category?: ProviderCategory;
  shouldShowApiKeyLink: boolean;
  websiteUrl: string;
  isPartner?: boolean;
  partnerPromotionKey?: string;

  // Base URL
  shouldShowSpeedTest: boolean;
  codexBaseUrl: string;
  onBaseUrlChange: (url: string) => void;
  isEndpointModalOpen: boolean;
  onEndpointModalToggle: (open: boolean) => void;
  onCustomEndpointsChange?: (endpoints: string[]) => void;
  autoSelect: boolean;
  onAutoSelectChange: (checked: boolean) => void;

  // Model Name
  shouldShowModelField?: boolean;
  modelName?: string;
  onModelNameChange?: (model: string) => void;

  // Speed Test Endpoints
  speedTestEndpoints: EndpointCandidate[];

  // OpenAI Official 双认证模式
  isOfficial?: boolean;
  authMode: "oauth" | "manual";
  onAuthModeChange: (mode: "oauth" | "manual") => void;
  onOauthLogin?: () => Promise<void> | void;
  oauthLoading?: boolean;
  oauthStatus?: string;
  hasOauthToken?: boolean;
}

export function CodexFormFields({
  providerId,
  codexApiKey,
  onApiKeyChange,
  category,
  shouldShowApiKeyLink,
  websiteUrl,
  isPartner,
  partnerPromotionKey,
  shouldShowSpeedTest,
  codexBaseUrl,
  onBaseUrlChange,
  isEndpointModalOpen,
  onEndpointModalToggle,
  onCustomEndpointsChange,
  autoSelect,
  onAutoSelectChange,
  shouldShowModelField = true,
  modelName = "",
  onModelNameChange,
  speedTestEndpoints,
  isOfficial = false,
  authMode,
  onAuthModeChange,
  onOauthLogin,
  oauthLoading = false,
  oauthStatus = "",
  hasOauthToken = false,
}: CodexFormFieldsProps) {
  const { t } = useTranslation();

  return (
    <>
      {/* OpenAI Official：OAuth / 手动 Token 双认证 */}
      {isOfficial ? (
        <div className="space-y-3">
          <div className="space-y-2">
            <label className="block text-sm font-medium text-foreground">
              {t("providerForm.codexAuthModeTitle", {
                defaultValue: "认证方式",
              })}
            </label>
            <div className="grid grid-cols-2 rounded-lg border border-border-default bg-muted/30 p-1">
              <button
                type="button"
                onClick={() => onAuthModeChange("oauth")}
                className={`rounded-md px-3 py-2 text-sm transition-colors ${
                  authMode === "oauth"
                    ? "bg-primary text-primary-foreground"
                    : "text-muted-foreground hover:text-foreground"
                }`}
              >
                Log in with ChatGPT
              </button>
              <button
                type="button"
                onClick={() => onAuthModeChange("manual")}
                className={`rounded-md px-3 py-2 text-sm transition-colors ${
                  authMode === "manual"
                    ? "bg-primary text-primary-foreground"
                    : "text-muted-foreground hover:text-foreground"
                }`}
              >
                {t("providerForm.codexManualToken", {
                  defaultValue: "手动输入 API Key",
                })}
              </button>
            </div>
          </div>

          {authMode === "oauth" ? (
            <div className="rounded-xl border border-border-default bg-muted/20 p-4 space-y-3">
              <button
                type="button"
                onClick={() => onOauthLogin?.()}
                disabled={oauthLoading}
                className="w-full px-4 py-2.5 rounded-lg bg-blue-600 text-white text-sm font-medium hover:bg-blue-700 disabled:opacity-60 disabled:cursor-not-allowed transition-colors"
              >
                {oauthLoading
                  ? t("providerForm.codexOauthLoggingIn", {
                      defaultValue: "登录中...",
                    })
                  : "Log in with ChatGPT"}
              </button>
              <p className="text-xs text-muted-foreground">
                {hasOauthToken
                  ? t("providerForm.codexOauthReady", {
                      defaultValue: "已获取 OAuth Token，可直接保存",
                    })
                  : t("providerForm.codexOauthHint", {
                      defaultValue:
                        "点击按钮跳转浏览器登录，完成后会自动回填 Token",
                    })}
              </p>
              {oauthStatus ? (
                <p className="text-xs text-blue-600 dark:text-blue-400">
                  {oauthStatus}
                </p>
              ) : null}
            </div>
          ) : (
            <ApiKeySection
              id="codexApiKey"
              label="API Key"
              value={codexApiKey}
              onChange={onApiKeyChange}
              category={category}
              shouldShowLink={shouldShowApiKeyLink}
              websiteUrl={websiteUrl}
              disabled={false}
              isPartner={isPartner}
              partnerPromotionKey={partnerPromotionKey}
              placeholder={{
                official: t("providerForm.codexOfficialManualHint", {
                  defaultValue: "请输入手动 Token",
                }),
                thirdParty: t("providerForm.codexApiKeyAutoFill", {
                  defaultValue: "输入 API Key，将自动填充到配置",
                }),
              }}
            />
          )}
        </div>
      ) : (
        /* 非官方供应商沿用原有 API Key 方式 */
        <ApiKeySection
          id="codexApiKey"
          label="API Key"
          value={codexApiKey}
          onChange={onApiKeyChange}
          category={category}
          shouldShowLink={shouldShowApiKeyLink}
          websiteUrl={websiteUrl}
          isPartner={isPartner}
          partnerPromotionKey={partnerPromotionKey}
          placeholder={{
            official: t("providerForm.codexOfficialNoApiKey", {
              defaultValue: "官方供应商无需 API Key",
            }),
            thirdParty: t("providerForm.codexApiKeyAutoFill", {
              defaultValue: "输入 API Key，将自动填充到配置",
            }),
          }}
        />
      )}

      {/* Codex Base URL 输入框 */}
      {shouldShowSpeedTest && (
        <EndpointField
          id="codexBaseUrl"
          label={t("codexConfig.apiUrlLabel")}
          value={codexBaseUrl}
          onChange={onBaseUrlChange}
          placeholder={t("providerForm.codexApiEndpointPlaceholder")}
          hint={t("providerForm.codexApiHint")}
          onManageClick={() => onEndpointModalToggle(true)}
        />
      )}

      {/* Codex Model Name 输入框 */}
      {shouldShowModelField && onModelNameChange && (
        <div className="space-y-2">
          <label
            htmlFor="codexModelName"
            className="block text-sm font-medium text-foreground"
          >
            {t("codexConfig.modelName", { defaultValue: "模型名称" })}
          </label>
          <input
            id="codexModelName"
            type="text"
            value={modelName}
            onChange={(e) => onModelNameChange(e.target.value)}
            placeholder={t("codexConfig.modelNamePlaceholder", {
              defaultValue: "例如: gpt-5-codex",
            })}
            className="w-full px-3 py-2 border border-border-default bg-background text-foreground rounded-lg text-sm focus:outline-none focus:ring-2 focus:ring-blue-500/20 dark:focus:ring-blue-400/20 transition-colors"
          />
          <p className="text-xs text-muted-foreground">
            {t("codexConfig.modelNameHint", {
              defaultValue: "指定使用的模型，将自动更新到 config.toml 中",
            })}
          </p>
        </div>
      )}

      {/* 端点测速弹窗 - Codex */}
      {shouldShowSpeedTest && isEndpointModalOpen && (
        <EndpointSpeedTest
          appId="codex"
          providerId={providerId}
          value={codexBaseUrl}
          onChange={onBaseUrlChange}
          initialEndpoints={speedTestEndpoints}
          visible={isEndpointModalOpen}
          onClose={() => onEndpointModalToggle(false)}
          autoSelect={autoSelect}
          onAutoSelectChange={onAutoSelectChange}
          onCustomEndpointsChange={onCustomEndpointsChange}
        />
      )}
    </>
  );
}

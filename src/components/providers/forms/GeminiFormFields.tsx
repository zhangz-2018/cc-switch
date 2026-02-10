import { useTranslation } from "react-i18next";
import { FormLabel } from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Download, Info, Loader2 } from "lucide-react";
import EndpointSpeedTest from "./EndpointSpeedTest";
import { ApiKeySection, EndpointField } from "./shared";
import type { ProviderCategory } from "@/types";

interface EndpointCandidate {
  url: string;
}

interface GeminiFormFieldsProps {
  providerId?: string;
  // API Key
  shouldShowApiKey: boolean;
  apiKey: string;
  onApiKeyChange: (key: string) => void;
  category?: ProviderCategory;
  shouldShowApiKeyLink: boolean;
  websiteUrl: string;
  isPartner?: boolean;
  partnerPromotionKey?: string;
  onImportAntigravitySession?: () => void;
  isImportingAntigravitySession?: boolean;

  // Base URL
  shouldShowSpeedTest: boolean;
  baseUrl: string;
  onBaseUrlChange: (url: string) => void;
  isEndpointModalOpen: boolean;
  onEndpointModalToggle: (open: boolean) => void;
  onCustomEndpointsChange: (endpoints: string[]) => void;
  autoSelect: boolean;
  onAutoSelectChange: (checked: boolean) => void;

  // Model
  shouldShowModelField: boolean;
  model: string;
  onModelChange: (value: string) => void;
  shouldShowModelsField: boolean;
  models: string;
  onModelsChange: (value: string) => void;

  // Speed Test Endpoints
  speedTestEndpoints: EndpointCandidate[];
}

export function GeminiFormFields({
  providerId,
  shouldShowApiKey,
  apiKey,
  onApiKeyChange,
  category,
  shouldShowApiKeyLink,
  websiteUrl,
  isPartner,
  partnerPromotionKey,
  onImportAntigravitySession,
  isImportingAntigravitySession = false,
  shouldShowSpeedTest,
  baseUrl,
  onBaseUrlChange,
  isEndpointModalOpen,
  onEndpointModalToggle,
  onCustomEndpointsChange,
  autoSelect,
  onAutoSelectChange,
  shouldShowModelField,
  model,
  onModelChange,
  shouldShowModelsField,
  models,
  onModelsChange,
  speedTestEndpoints,
}: GeminiFormFieldsProps) {
  const { t } = useTranslation();

  // 检测是否为 Google 官方（使用 OAuth）
  const isGoogleOfficial =
    partnerPromotionKey?.toLowerCase() === "google-official";
  const isAntigravityOfficial =
    partnerPromotionKey?.toLowerCase() === "antigravity";

  return (
    <>
      {/* Google OAuth 提示 */}
      {isGoogleOfficial && (
        <div className="rounded-lg border border-blue-200 bg-blue-50 p-4 dark:border-blue-800 dark:bg-blue-950">
          <div className="flex gap-3">
            <Info className="h-5 w-5 flex-shrink-0 text-blue-600 dark:text-blue-400" />
            <div className="space-y-1">
              <p className="text-sm font-medium text-blue-900 dark:text-blue-100">
                {t("provider.form.gemini.oauthTitle", {
                  defaultValue: "OAuth 认证模式",
                })}
              </p>
              <p className="text-sm text-blue-700 dark:text-blue-300">
                {t("provider.form.gemini.oauthHint", {
                  defaultValue:
                    "Google 官方使用 OAuth 个人认证，无需填写 API Key。首次使用时会自动打开浏览器进行登录。",
                })}
              </p>
            </div>
          </div>
        </div>
      )}

      {/* Antigravity 官方账号导入 */}
      {isAntigravityOfficial && (
        <div className="rounded-lg border border-sky-200 bg-sky-50 p-4 dark:border-sky-800 dark:bg-sky-950">
          <div className="flex flex-col gap-3">
            <div className="flex gap-3">
              <Info className="h-5 w-5 flex-shrink-0 text-sky-600 dark:text-sky-400" />
              <div className="space-y-1">
                <p className="text-sm font-medium text-sky-900 dark:text-sky-100">
                  {t("provider.form.gemini.antigravityTitle", {
                    defaultValue: "Antigravity 官方账号模式",
                  })}
                </p>
                <p className="text-sm text-sky-700 dark:text-sky-300">
                  {t("provider.form.gemini.antigravityHint", {
                    defaultValue:
                      "可一键导入本机 Antigravity 已登录账号。切换到该供应商时会自动同步账号并重启 Antigravity 客户端。",
                  })}
                </p>
              </div>
            </div>
            <div>
              <Button
                type="button"
                variant="outline"
                onClick={onImportAntigravitySession}
                disabled={isImportingAntigravitySession}
                className="border-sky-300 text-sky-700 hover:bg-sky-100 dark:border-sky-700 dark:text-sky-300 dark:hover:bg-sky-900/40"
              >
                {isImportingAntigravitySession ? (
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                ) : (
                  <Download className="mr-2 h-4 w-4" />
                )}
                {t("provider.form.gemini.importAntigravitySession", {
                  defaultValue: "导入当前 Antigravity 账号",
                })}
              </Button>
            </div>
          </div>
        </div>
      )}

      {/* API Key 输入框 */}
      {shouldShowApiKey && !isGoogleOfficial && !isAntigravityOfficial && (
        <ApiKeySection
          value={apiKey}
          onChange={onApiKeyChange}
          category={category}
          shouldShowLink={shouldShowApiKeyLink}
          websiteUrl={websiteUrl}
          isPartner={isPartner}
          partnerPromotionKey={partnerPromotionKey}
        />
      )}

      {/* Base URL 输入框（统一使用与 Codex 相同的样式与交互） */}
      {shouldShowSpeedTest && (
        <EndpointField
          id="baseUrl"
          label={t("providerForm.apiEndpoint", { defaultValue: "API 端点" })}
          value={baseUrl}
          onChange={onBaseUrlChange}
          placeholder={t("providerForm.apiEndpointPlaceholder", {
            defaultValue: "https://your-api-endpoint.com/",
          })}
          onManageClick={() => onEndpointModalToggle(true)}
        />
      )}

      {/* Model 输入框 */}
      {shouldShowModelField && (
        <div>
          <FormLabel htmlFor="gemini-model">
            {t("provider.form.gemini.model", { defaultValue: "模型" })}
          </FormLabel>
          <Input
            id="gemini-model"
            value={model}
            onChange={(e) => onModelChange(e.target.value)}
            placeholder="gemini-3-pro-preview"
          />
        </div>
      )}

      {/* 多模型输入框（逗号分隔） */}
      {shouldShowModelsField && (
        <div>
          <FormLabel htmlFor="gemini-models">
            {t("provider.form.gemini.models", {
              defaultValue: "多模型（逗号分隔）",
            })}
          </FormLabel>
          <Input
            id="gemini-models"
            value={models}
            onChange={(e) => onModelsChange(e.target.value)}
            placeholder="gemini-2.5-pro,gemini-2.5-flash"
          />
          <p className="mt-1 text-xs text-muted-foreground">
            {t("provider.form.gemini.modelsHint", {
              defaultValue: "用于支持多个模型自动切换，留空则使用单模型字段",
            })}
          </p>
        </div>
      )}

      {/* 端点测速弹窗 */}
      {shouldShowSpeedTest && isEndpointModalOpen && (
        <EndpointSpeedTest
          appId="gemini"
          providerId={providerId}
          value={baseUrl}
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

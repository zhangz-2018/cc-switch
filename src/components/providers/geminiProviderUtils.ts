import type { AppId } from "@/lib/api";
import type { Provider } from "@/types";

const GOOGLE_OFFICIAL_PARTNER_KEY = "google-official";
const ANTIGRAVITY_PARTNER_KEY = "antigravity";
const GOOGLE_OAUTH_ACCESS_TOKEN_KEY = "GOOGLE_OAUTH_ACCESS_TOKEN";
const GEMINI_API_KEY = "GEMINI_API_KEY";

const toLower = (value?: string | null): string => (value ?? "").toLowerCase();

const isNonEmptyString = (value: unknown): value is string =>
  typeof value === "string" && value.trim().length > 0;

const getProviderSettingsConfig = (
  provider: Provider,
): Record<string, unknown> => {
  const rawConfig = provider.settingsConfig as unknown;
  if (rawConfig && typeof rawConfig === "object") {
    return rawConfig as Record<string, unknown>;
  }

  // 兼容异常数据：settingsConfig 可能被序列化成 JSON 字符串
  if (typeof rawConfig === "string" && rawConfig.trim().startsWith("{")) {
    try {
      const parsed = JSON.parse(rawConfig) as Record<string, unknown>;
      if (parsed && typeof parsed === "object") {
        return parsed;
      }
    } catch {
      return {};
    }
  }

  return {};
};

const getGeminiEnv = (provider: Provider): Record<string, unknown> => {
  const config = getProviderSettingsConfig(provider);
  const env = config.env;
  if (env && typeof env === "object") {
    return env as Record<string, unknown>;
  }
  return {};
};

const extractOauthTokenFromGeminiApiKey = (
  value: unknown,
): string | undefined => {
  if (!isNonEmptyString(value)) return undefined;

  const token = value.trim();
  if (token.startsWith("ya29.")) return token;

  // 兼容历史结构：将 OAuth 信息序列化到 GEMINI_API_KEY 的 JSON 字符串
  if (token.startsWith("{")) {
    try {
      const parsed = JSON.parse(token) as Record<string, unknown>;
      const candidates = [
        parsed.accessToken,
        parsed.access_token,
        parsed.token,
      ];
      const oauthToken = candidates.find(
        (candidate): candidate is string =>
          isNonEmptyString(candidate) && candidate.trim().startsWith("ya29."),
      );
      return oauthToken?.trim();
    } catch {
      return undefined;
    }
  }

  return undefined;
};

export const isGeminiAntigravityProvider = (
  provider: Provider,
  appId: AppId,
): boolean => {
  if (appId !== "gemini") return false;

  if (toLower(provider.meta?.partnerPromotionKey) === ANTIGRAVITY_PARTNER_KEY) {
    return true;
  }

  if (toLower(provider.name).includes("antigravity")) {
    return true;
  }

  if (toLower(provider.websiteUrl).includes("antigravity")) {
    return true;
  }

  const baseUrl = getGeminiEnv(provider).GOOGLE_GEMINI_BASE_URL;
  return (
    typeof baseUrl === "string" &&
    baseUrl.toLowerCase().includes("daily-cloudcode-pa.sandbox.googleapis.com")
  );
};

export const isGeminiGoogleOfficialProvider = (
  provider: Provider,
  appId: AppId,
): boolean => {
  if (appId !== "gemini") return false;

  if (toLower(provider.meta?.partnerPromotionKey) === GOOGLE_OFFICIAL_PARTNER_KEY) {
    return true;
  }

  const providerName = toLower(provider.name).trim();
  if (providerName === "google" || providerName.startsWith("google ")) {
    return true;
  }

  return toLower(provider.websiteUrl).includes("ai.google.dev");
};

export const hasGeminiGoogleOauthToken = (provider: Provider): boolean => {
  const config = getProviderSettingsConfig(provider);
  const env = getGeminiEnv(provider);
  if (isNonEmptyString(env[GOOGLE_OAUTH_ACCESS_TOKEN_KEY])) {
    return true;
  }
  // 兼容异常结构：token 写在 settingsConfig 根层
  if (isNonEmptyString(config[GOOGLE_OAUTH_ACCESS_TOKEN_KEY])) {
    return true;
  }
  return !!extractOauthTokenFromGeminiApiKey(env[GEMINI_API_KEY]);
};

export const isGeminiUsageProvider = (
  provider: Provider,
  appId: AppId,
): boolean => {
  if (appId !== "gemini") return false;

  if (provider.meta?.usage_script?.enabled === true) {
    return true;
  }

  if (isGeminiAntigravityProvider(provider, appId)) {
    return true;
  }

  return (
    isGeminiGoogleOfficialProvider(provider, appId) &&
    hasGeminiGoogleOauthToken(provider)
  );
};

export const isGeminiUsageCandidateProvider = (
  provider: Provider,
  appId: AppId,
): boolean => {
  if (appId !== "gemini") return false;

  if (provider.meta?.usage_script?.enabled === true) {
    return true;
  }

  if (isGeminiAntigravityProvider(provider, appId)) {
    return true;
  }

  return isGeminiGoogleOfficialProvider(provider, appId);
};

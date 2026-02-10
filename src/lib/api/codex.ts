import { invoke } from "@tauri-apps/api/core";

export interface CodexOAuthDeviceFlowResponse {
  deviceCode: string;
  userCode: string;
  verificationUri: string;
  verificationUriComplete?: string;
  expiresIn: number;
  interval: number;
}

export interface CodexOAuthPollResponse {
  status: "pending" | "success" | "error";
  authJson?: Record<string, unknown>;
  email?: string;
  error?: string;
  errorDescription?: string;
}

export interface CodexQuotaWindow {
  usedPercent: number;
  limitWindowSeconds: number;
  resetAt: number;
}

export interface CodexQuotaResponse {
  planType?: string;
  fiveHour?: CodexQuotaWindow;
  weekly?: CodexQuotaWindow;
  fetchedAt: number;
}

export const codexApi = {
  initOAuthDeviceFlow: async (): Promise<CodexOAuthDeviceFlowResponse> => {
    return invoke("codex_oauth_init_device_flow");
  },

  pollOAuthToken: async (
    deviceCode: string,
  ): Promise<CodexOAuthPollResponse> => {
    return invoke("codex_oauth_poll_token", { deviceCode });
  },

  getQuota: async (providerId: string): Promise<CodexQuotaResponse> => {
    return invoke("codex_get_quota", { providerId });
  },

  restartCli: async (): Promise<boolean> => {
    return invoke("restart_codex_cli");
  },

  restartApp: async (): Promise<boolean> => {
    return invoke("restart_codex_app");
  },
};

import { invoke } from "@tauri-apps/api/core";
import type { AppId } from "./types";

export interface AntigravityImportedSession {
  email: string;
  accessToken: string;
  refreshToken: string;
  expiresAt: number;
  projectId?: string;
}

export interface AntigravityModelQuota {
  name: string;
  remainingPercent: number;
  usedPercent: number;
  resetTime?: string;
}

export interface AntigravityQuotaResponse {
  projectId: string;
  subscriptionTier?: string;
  models: AntigravityModelQuota[];
  fetchedAt: number;
}

export const antigravityApi = {
  async startLogin(): Promise<boolean> {
    return invoke("antigravity_start_login");
  },

  async importCurrentSession(): Promise<AntigravityImportedSession> {
    return invoke("antigravity_import_current_session");
  },

  async getQuota(
    providerId: string,
    appId: AppId = "gemini",
  ): Promise<AntigravityQuotaResponse> {
    return invoke("antigravity_get_quota", { providerId, app: appId });
  },
};

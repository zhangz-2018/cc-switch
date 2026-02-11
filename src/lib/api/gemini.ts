import { invoke } from "@tauri-apps/api/core";

export interface GeminiOAuthInitResponse {
  deviceCode: string;
  verificationUri: string;
  verificationUriComplete?: string;
  expiresIn: number;
  interval: number;
}

export interface GeminiOAuthPollResponse {
  status: "pending" | "success" | "error";
  email?: string;
  accessToken?: string;
  refreshToken?: string;
  expiresAt?: number;
  error?: string;
  errorDescription?: string;
}

export const geminiApi = {
  initOAuthLogin: async (): Promise<GeminiOAuthInitResponse> => {
    return invoke("gemini_oauth_init_login");
  },

  pollOAuthToken: async (
    deviceCode: string,
  ): Promise<GeminiOAuthPollResponse> => {
    return invoke("gemini_oauth_poll_token", { deviceCode });
  },
};

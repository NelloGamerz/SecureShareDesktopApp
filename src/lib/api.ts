import axios, { AxiosError, type InternalAxiosRequestConfig } from "axios";
import { env } from "@/lib/env";
import { error } from "@tauri-apps/plugin-log";
import { getDeviceIdentifier } from "@/services/getDeviceInfo";

const api = axios.create({
  baseURL: env.apiBaseUrl,
  timeout: 30_000,
  headers: {
    "Content-Type": "application/json",
  },
});

let tokenGetter: (() => Promise<string | null>) | null = null;

/**
 * Called once from `AxiosProvider` to wire the desktop session token into
 * axios. The getter reads from the Rust `AuthState`, so it always returns the
 * current token and refreshes it when it is near expiry.
 */
export function setTokenGetter(getter: () => Promise<string | null>) {
  tokenGetter = getter;
}

api.interceptors.request.use(async (config: InternalAxiosRequestConfig) => {
  if (tokenGetter) {
    try {
      const token = await tokenGetter();

      if (token) {
        config.headers.Authorization = `Bearer ${token}`;
      }

      const deviceIdentifier = await getDeviceIdentifier();

      config.headers["X-Device-Id"] = deviceIdentifier;
    } catch (err) {
      // Log the failure only. Never log the token or the header value.
      await error(`Failed to attach the authorization header: ${String(err)}`);
    }
  }

  return config;
});

api.interceptors.response.use(
  (response) => response,
  (error: AxiosError<{ message?: string; error?: string }>) => {
    const status = error.response?.status;
    const message =
      error.response?.data?.error ??
      error.response?.data?.message ??
      error.message ??
      "Unexpected error";

    if (status === 401) {
      return Promise.reject(
        Object.assign(
          new Error("Your session has expired. Please sign in again."),
          {
            status,
            isAuthError: true,
          },
        ),
      );
    }

    return Promise.reject(
      Object.assign(new Error(message), { status, isAxiosError: true }),
    );
  },
);

export { api };
export default api;

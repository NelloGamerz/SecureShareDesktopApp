import axios, { AxiosError, type InternalAxiosRequestConfig } from "axios";
import { env } from "@/lib/env";
import { info, error } from "@tauri-apps/plugin-log";

const api = axios.create({
  baseURL: env.apiBaseUrl,
  timeout: 30_000,
  headers: {
    "Content-Type": "application/json",
  },
});

let tokenGetter: (() => Promise<string | null>) | null = null;

/** Called once from the AxiosProvider to wire Clerk's getToken into axios. */
export function setTokenGetter(getter: () => Promise<string | null>) {
  tokenGetter = getter;
}

// api.interceptors.request.use(async (config: InternalAxiosRequestConfig) => {
//   // Attach the Clerk session JWT when available.
//   if (tokenGetter) {
//     try {
//       const token = await tokenGetter();
//       if (token) {
//         config.headers.Authorization = `Bearer ${token}`;
//       }
//     } catch {
//       // token unavailable; continue without auth header
//     }
//   }
//   return config;
// });

api.interceptors.request.use(async (config: InternalAxiosRequestConfig) => {
  if (tokenGetter) {
    try {
      const token = await tokenGetter();

      await info(`Clerk token exists: ${!!token}`);
      await info(`Clerk token length: ${token?.length ?? 0}`);

      if (token) {
        config.headers.Authorization = `Bearer ${token}`;

        await info("Authorization header attached");
      } else {
        await info("No Clerk token received");
      }
    } catch (err) {
      await error(`Failed to get Clerk token: ${String(err)}`);
    }
  } else {
    await info("tokenGetter is null");
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

import { loginWithTauri, logoutFromTauri } from '@/api/tauri';
import { getDeviceInfo } from './getDeviceInfo';

export async function authenticateWithTauri(token: string) {
  await loginWithTauri(token, await getDeviceInfo());
}

export async function deauthenticateWithTauri() {
  await logoutFromTauri();
}

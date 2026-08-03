import { Store } from "@tauri-apps/plugin-store";
import { v4 as uuidv4 } from "uuid";
import { detectDeviceType } from "@/api/tauri";

export type DeviceType = "DESKTOP" | "MOBILE" | "TABLET" | "LAPTOP" | "UNKNOWN";

export type OperatingSystem = "WINDOWS" | "MACOS" | "LINUX" | "IOS" | "ANDROID";

export async function getDeviceInfo() {
  // const store = await Store.load("device.json");

  // let deviceIdentifier = await store.get<string>("deviceIdentifier");

  // if (!deviceIdentifier) {
  //   deviceIdentifier = uuidv4();

  //   await store.set("deviceIdentifier", deviceIdentifier);
  //   await store.save();
  // }

  const deviceIdentifier = await getDeviceIdentifier();

  const detectedType = await detectDeviceType();
  return {
    deviceName: getDeviceName(),
    deviceIdentifier,
    deviceType: mapDeviceType(detectedType),
    operatingSystem: getOperatingSystem(),
    appVersion: getAppVersion(),
  };
}

export async function getDeviceIdentifier(): Promise<string> {
  const store = await Store.load("device.json");

  let deviceIdentifier = await store.get<string>("deviceIdentifier");

  if (!deviceIdentifier) {
    deviceIdentifier = uuidv4();

    await store.set("deviceIdentifier", deviceIdentifier);

    await store.save();
  }

  return deviceIdentifier;
}

function mapDeviceType(type: string): DeviceType {
  switch (type) {
    case "LAPTOP":
      return "LAPTOP";

    case "DESKTOP":
      return "DESKTOP";

    case "MOBILE":
      return "MOBILE";

    case "TABLET":
      return "TABLET";

    default:
      return "UNKNOWN";
  }
}

function getDeviceName(): string {
  return navigator.platform || "Unknown Desktop";
}

// function getDeviceType(): DeviceType {
//   return "DESKTOP";
// }

function getOperatingSystem(): OperatingSystem {
  const userAgent = navigator.userAgent.toLowerCase();
  const platform = navigator.platform.toLowerCase();

  if (/iphone|ipad|ipod/.test(userAgent)) {
    return "IOS";
  }

  if (/android/.test(userAgent)) {
    return "ANDROID";
  }

  if (platform.includes("win")) {
    return "WINDOWS";
  }

  if (platform.includes("mac")) {
    return "MACOS";
  }

  if (platform.includes("linux")) {
    return "LINUX";
  }

  return "WINDOWS";
}

function getAppVersion(): string {
  return "1.0.0";
}

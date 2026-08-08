// Razorpay Checkout Service
// Provides reusable functions to open Razorpay subscription and order checkouts.

export class RazorpayLoadError extends Error {
  constructor(message = 'Unable to load Razorpay Checkout script.') {
    super(message);
    this.name = 'RazorpayLoadError';
  }
}

export class RazorpayUnavailableError extends Error {
  constructor(message = 'Razorpay Checkout is not available in this environment.') {
    super(message);
    this.name = 'RazorpayUnavailableError';
  }
}

export interface RazorpayHandlerResponse {
  razorpay_payment_id?: string;
  razorpay_order_id?: string;
  razorpay_subscription_id?: string;
  razorpay_signature?: string;
}

export interface RazorpayTheme {
  color?: string;
}

export interface RazorpayPrefill {
  name?: string;
  email?: string;
  contact?: string;
}

export interface SubscriptionCheckoutOptions {
  key: string;
  subscriptionId: string;
  name?: string;
  description?: string;
  prefill?: RazorpayPrefill;
  notes?: Record<string, string>;
  theme?: RazorpayTheme;
  onSuccess?: (res: SubscriptionCheckoutResponse) => void;
  onDismiss?: () => void;
}

export interface SubscriptionCheckoutResponse {
  razorpay_payment_id: string;
  razorpay_subscription_id: string;
  razorpay_signature: string;
}

export interface OrderCheckoutOptions {
  key: string;
  orderId: string;
  amount?: number;
  currency?: string;
  name?: string;
  description?: string;
  prefill?: RazorpayPrefill;
  notes?: Record<string, string>;
  theme?: RazorpayTheme;
  onSuccess?: (res: OrderCheckoutResponse) => void;
  onDismiss?: () => void;
}

export interface OrderCheckoutResponse {
  razorpay_payment_id: string;
  razorpay_order_id: string;
  razorpay_signature: string;
}

type RazorpayModal = { ondismiss?: () => void };

type RazorpayOptions = {
  key: string;
  amount?: number;
  currency?: string;
  name?: string;
  description?: string;
  order_id?: string;
  subscription_id?: string;
  handler: (response: RazorpayHandlerResponse) => void;
  modal?: RazorpayModal;
  prefill?: RazorpayPrefill;
  notes?: Record<string, string>;
  theme?: RazorpayTheme;
};

type RazorpayInstance = {
  open: () => void;
};

declare global {
  interface Window {
    Razorpay?: new (opts: RazorpayOptions) => RazorpayInstance;
  }
}

const SCRIPT_ID = 'razorpay-checkout-script';
const SCRIPT_SRC = 'https://checkout.razorpay.com/v1/checkout.js';

function loadScript(): Promise<void> {
  return new Promise((resolve, reject) => {
    if (typeof window === 'undefined' || typeof document === 'undefined') {
      reject(new RazorpayUnavailableError('Document/window not available in this environment.'));
      return;
    }

    if (window.Razorpay) {
      resolve();
      return;
    }

    const existing = document.getElementById(SCRIPT_ID) as HTMLScriptElement | null;
    if (existing) {
      existing.addEventListener('load', () => resolve(), { once: true });
      existing.addEventListener('error', () => reject(new RazorpayLoadError()), { once: true });
      return;
    }

    const script = document.createElement('script');
    script.id = SCRIPT_ID;
    script.src = SCRIPT_SRC;
    script.async = true;
    script.onload = () => resolve();
    script.onerror = () => reject(new RazorpayLoadError());
    document.body.appendChild(script);
  });
}

async function ensureRazorpay(): Promise<void> {
  await loadScript();
  if (!window.Razorpay) {
    throw new RazorpayUnavailableError('Razorpay Checkout did not initialize.');
  }
}

export async function openSubscriptionCheckout(options: SubscriptionCheckoutOptions): Promise<SubscriptionCheckoutResponse> {
  await ensureRazorpay();

  return new Promise<SubscriptionCheckoutResponse>((resolve, reject) => {
    const opts: RazorpayOptions = {
      key: options.key,
      subscription_id: options.subscriptionId,
      name: options.name,
      description: options.description,
      handler: (resp) => {
        const paymentId = resp.razorpay_payment_id;
        const subscriptionId = resp.razorpay_subscription_id;
        const signature = resp.razorpay_signature;

        if (!paymentId || !subscriptionId || !signature) {
          reject(new Error('Invalid response from Razorpay subscription handler.'));
          return;
        }

        const result: SubscriptionCheckoutResponse = {
          razorpay_payment_id: paymentId,
          razorpay_subscription_id: subscriptionId,
          razorpay_signature: signature,
        };

        try {
          options.onSuccess?.(result);
        } catch {
          // ignore onSuccess errors
        }

        resolve(result);
      },
      modal: {
        ondismiss: () => {
          try {
            options.onDismiss?.();
          } catch {
            // ignore
          }
          reject(new Error('Razorpay subscription checkout was dismissed by the user.'));
        },
      },
      prefill: options.prefill,
      notes: options.notes,
      theme: options.theme,
    };

    try {
      const RazorpayCtor = window.Razorpay;
      if (!RazorpayCtor) {
        reject(new RazorpayUnavailableError('Razorpay Checkout is not available.'));
        return;
      }

      const instance = new RazorpayCtor(opts as RazorpayOptions);
      instance.open();
    } catch (err) {
      reject(err instanceof Error ? err : new Error('Failed to open Razorpay subscription checkout.'));
    }
  });
}

export async function openOrderCheckout(options: OrderCheckoutOptions): Promise<OrderCheckoutResponse> {
  await ensureRazorpay();

  return new Promise<OrderCheckoutResponse>((resolve, reject) => {
    const opts: RazorpayOptions = {
      key: options.key,
      amount: options.amount,
      currency: options.currency,
      name: options.name,
      description: options.description,
      order_id: options.orderId,
      handler: (resp) => {
        const paymentId = resp.razorpay_payment_id;
        const orderId = resp.razorpay_order_id;
        const signature = resp.razorpay_signature;

        if (!paymentId || !orderId || !signature) {
          reject(new Error('Invalid response from Razorpay order handler.'));
          return;
        }

        const result: OrderCheckoutResponse = {
          razorpay_payment_id: paymentId,
          razorpay_order_id: orderId,
          razorpay_signature: signature,
        };

        try {
          options.onSuccess?.(result);
        } catch {
          // ignore
        }

        resolve(result);
      },
      modal: {
        ondismiss: () => {
          try {
            options.onDismiss?.();
          } catch {
            // ignore
          }
          reject(new Error('Razorpay order checkout was dismissed by the user.'));
        },
      },
      prefill: options.prefill,
      notes: options.notes,
      theme: options.theme,
    };

    try {
      const RazorpayCtor = window.Razorpay;
      if (!RazorpayCtor) {
        reject(new RazorpayUnavailableError('Razorpay Checkout is not available.'));
        return;
      }

      const instance = new RazorpayCtor(opts as RazorpayOptions);
      instance.open();
    } catch (err) {
      reject(err instanceof Error ? err : new Error('Failed to open Razorpay order checkout.'));
    }
  });
}

export default {
  openSubscriptionCheckout,
  openOrderCheckout,
};

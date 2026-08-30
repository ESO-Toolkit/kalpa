/**
 * Collects the allow-listed environment details a support ticket needs.
 *
 * Each value comes from a trusted local Tauri API or the web view's own
 * user-agent, is normalized against the bounded rules in `support-report.ts`,
 * and degrades to `unknown` instead of guessing. See the `SupportEnvironment`
 * documentation there for why each field is present and what is deliberately
 * never collected — in particular this module must never reach for
 * `hostname()`, `locale()`, environment variables, or any path.
 */
import { getTauriVersion } from "@tauri-apps/api/app";
import { arch, version } from "@tauri-apps/plugin-os";

import {
  normalizeArchitecture,
  normalizeOsVersion,
  normalizeRuntimeVersion,
  normalizeWebviewLabel,
  SUPPORT_UNKNOWN,
  UNKNOWN_SUPPORT_ENVIRONMENT,
  type SupportEnvironment,
} from "@/lib/support-report";

function safely<T>(read: () => T): T | undefined {
  try {
    return read();
  } catch {
    return undefined;
  }
}

export async function collectSupportEnvironment(): Promise<SupportEnvironment> {
  const tauri = await getTauriVersion().catch(() => SUPPORT_UNKNOWN);
  return {
    osVersion: normalizeOsVersion(safely(version)),
    arch: normalizeArchitecture(safely(arch)),
    tauri: normalizeRuntimeVersion(tauri),
    webview: normalizeWebviewLabel(safely(() => navigator.userAgent)),
  };
}

export { UNKNOWN_SUPPORT_ENVIRONMENT };

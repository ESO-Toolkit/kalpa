import type { Env } from "./types";

const RATE_LIMIT_PREFIX = "rl:";

interface RateLimitConfig {
  windowSeconds: number;
  maxRequests: number;
}

const RATE_LIMITS: Record<string, RateLimitConfig> = {
  read: { windowSeconds: 60, maxRequests: 60 },
  write: { windowSeconds: 60, maxRequests: 10 },
  vote: { windowSeconds: 60, maxRequests: 20 },
};

export async function checkRateLimit(
  env: Env,
  ip: string,
  action: keyof typeof RATE_LIMITS,
): Promise<{ allowed: boolean; retryAfter?: number }> {
  const config = RATE_LIMITS[action];
  const window = Math.floor(Date.now() / (config.windowSeconds * 1000));
  const key = `${RATE_LIMIT_PREFIX}${action}:${ip}:${window}`;

  const current = await env.ESO_PACKS.get(key);
  const count = current ? parseInt(current, 10) : 0;

  if (count >= config.maxRequests) {
    const retryAfter = config.windowSeconds - Math.floor((Date.now() / 1000) % config.windowSeconds);
    return { allowed: false, retryAfter };
  }

  await env.ESO_PACKS.put(key, String(count + 1), {
    expirationTtl: config.windowSeconds * 2,
  });

  return { allowed: true };
}

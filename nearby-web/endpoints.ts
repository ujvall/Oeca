// oeca: where the page connects. A production build (nuxt generate) uses
// LocalSend's public servers unless told otherwise. Anything else (the dev
// server, tests) never reaches them by default: signaling goes to a dead
// loopback port and there is no STUN, until SIGNALING_URL / STUN_SERVERS say
// otherwise, explicitly.

export const PUBLIC_SIGNALING = "wss://public.localsend.org/v1/ws";
export const PUBLIC_STUN = "stun:stun.localsend.org:3478";
/** Port 9 (discard): nothing listens, the connection just fails. */
export const LOOPBACK_SIGNALING = "ws://127.0.0.1:9/";

export function endpoints(env: Record<string, string | undefined>): { signalingUrl: string; stunServers: string[] } {
  const production = env.NODE_ENV === "production";
  const signalingUrl = env.SIGNALING_URL ?? (production ? PUBLIC_SIGNALING : LOOPBACK_SIGNALING);
  // STUN_SERVERS="" (empty): none, for loopback test builds
  const stunServers = (env.STUN_SERVERS ?? (production ? PUBLIC_STUN : "")).split(",").filter(Boolean);
  return { signalingUrl, stunServers };
}

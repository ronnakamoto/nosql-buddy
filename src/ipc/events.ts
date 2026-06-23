// Typed event listeners for the frontend.
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface ConnectionOpenedPayload {
  connectionId: string;
  profileId: string;
  name: string;
}

export interface ConnectionClosedPayload {
  connectionId: string;
  profileId: string;
  at: string;
}

export async function onConnectionOpened(
  handler: (payload: ConnectionOpenedPayload) => void,
): Promise<UnlistenFn> {
  return listen<ConnectionOpenedPayload>("connection-opened", (event) =>
    handler(event.payload),
  );
}

export async function onConnectionClosed(
  handler: (payload: ConnectionClosedPayload) => void,
): Promise<UnlistenFn> {
  return listen<ConnectionClosedPayload>("connection-closed", (event) =>
    handler(event.payload),
  );
}

export async function onMenuAction(
  handler: (action: string) => void,
): Promise<UnlistenFn> {
  return listen<string>("menu-action", (event) => handler(event.payload));
}

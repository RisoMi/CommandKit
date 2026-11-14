/**
 * IPC 桥：Tauri 环境走真实 invoke / listen；纯浏览器打开时走 mock（仅视觉走查用）。
 */
import { invoke as tauriInvoke } from '@tauri-apps/api/core';
import { listen as tauriListen } from '@tauri-apps/api/event';
import * as mock from './mock';

export const inTauri =
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in (window as unknown as Record<string, unknown>);

export function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return inTauri ? tauriInvoke<T>(cmd, args) : mock.invoke<T>(cmd, args);
}

export function listen<T>(event: string, cb: (ev: { payload: T }) => void): Promise<() => void> {
  if (inTauri) return tauriListen<T>(event, cb);
  return mock.listen(event, cb as unknown as (ev: { payload: unknown }) => void);
}

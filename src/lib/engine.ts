import { invoke } from "@tauri-apps/api/core";

export interface SysInfo {
  os: string;
  arch: string;
  cpuCount: number;
  memoryGB: number;
  swift: string;
}

export async function engineCall<T>(cmd: string, params?: object): Promise<T> {
  return invoke<T>("engine_call", { cmd, params: params ?? null });
}

export function enginePing(): Promise<{ pong: boolean }> {
  return engineCall<{ pong: boolean }>("ping");
}

export function engineSysInfo(): Promise<SysInfo> {
  return engineCall<SysInfo>("sysinfo");
}

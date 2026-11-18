import { open, save } from '@tauri-apps/plugin-dialog';
import { inTauri } from './bridge';

const JSON_FILTER = [{ name: 'JSON 配置', extensions: ['json'] }];

/** 选一个保存路径；浏览器演示模式返回 null（没有原生对话框） */
export async function pickSavePath(defaultName: string): Promise<string | null> {
  if (!inTauri) return null;
  const path = await save({ defaultPath: defaultName, filters: JSON_FILTER });
  return typeof path === 'string' ? path : null;
}

/** 选一个已有文件；浏览器演示模式返回 null */
export async function pickOpenPath(): Promise<string | null> {
  if (!inTauri) return null;
  const picked = await open({ multiple: false, directory: false, filters: JSON_FILTER });
  return typeof picked === 'string' ? picked : null;
}

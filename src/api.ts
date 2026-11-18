import { invoke } from './bridge';
import type {
  Component,
  ImportResult,
  LogChunk,
  ProcessDetail,
  Project,
  Settings,
  Snapshot,
  Template,
} from './types';

export const api = {
  bootstrap: () => invoke<Snapshot>('bootstrap'),

  // 项目（组件分组）
  saveProject: (project: Project) => invoke<Snapshot>('save_project', { project }),
  deleteProject: (id: string) => invoke<Snapshot>('delete_project', { id }),

  // 组件（可启动单元）
  saveComponent: (component: Component) => invoke<Snapshot>('save_component', { component }),
  deleteComponent: (id: string) => invoke<Snapshot>('delete_component', { id }),

  exportComponents: (ids: string[], path: string) =>
    invoke<string>('export_components', { ids, path }),
  importComponents: (path: string) => invoke<ImportResult>('import_components', { path }),

  saveTemplate: (template: Template) => invoke<Snapshot>('save_template', { template }),
  deleteTemplate: (id: string) => invoke<Snapshot>('delete_template', { id }),

  saveSettings: (settings: Settings) => invoke<Snapshot>('save_settings', { settings }),

  runComponent: (id: string) => invoke<void>('run_component', { id }),
  stopComponent: (id: string) => invoke<void>('stop_component', { id }),
  restartComponent: (id: string) => invoke<void>('restart_component', { id }),

  runMany: (ids: string[]) => invoke<void>('run_many', { ids }),
  stopMany: (ids: string[]) => invoke<void>('stop_many_cmd', { ids }),
  restartMany: (ids: string[]) => invoke<void>('restart_many', { ids }),
  cancelBulk: () => invoke<void>('cancel_bulk_cmd'),

  runTemplate: (id: string) => invoke<void>('run_template', { id }),

  sendInput: (id: string, data: string) => invoke<void>('send_input_cmd', { id, data }),
  sessionLog: (id: string) => invoke<LogChunk[]>('session_log', { id }),
  clearLog: (id: string) => invoke<void>('clear_session_log', { id }),

  processDetail: (id: string) => invoke<ProcessDetail>('process_detail', { id }),

  exportLog: (id: string) => invoke<string>('export_log', { id }),
  exportSnapshot: () => invoke<string>('export_snapshot'),

  openDataDir: () => invoke<void>('open_data_dir'),
  openWorkdir: (id: string) => invoke<void>('open_workdir', { id }),
  openExternalTerminal: (id: string) => invoke<void>('open_external_terminal', { id }),
};

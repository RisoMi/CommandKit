import { invoke } from './bridge';
import type {
  Category,
  LogChunk,
  ProcessDetail,
  Project,
  Settings,
  Snapshot,
  Template,
} from './types';

export const api = {
  bootstrap: () => invoke<Snapshot>('bootstrap'),

  saveProject: (project: Project) => invoke<Snapshot>('save_project', { project }),
  deleteProject: (id: string) => invoke<Snapshot>('delete_project', { id }),

  saveCategory: (category: Category) => invoke<Snapshot>('save_category', { category }),
  deleteCategory: (id: string) => invoke<Snapshot>('delete_category', { id }),

  saveTemplate: (template: Template) => invoke<Snapshot>('save_template', { template }),
  deleteTemplate: (id: string) => invoke<Snapshot>('delete_template', { id }),

  saveSettings: (settings: Settings) => invoke<Snapshot>('save_settings', { settings }),

  runProject: (id: string) => invoke<void>('run_project', { id }),
  stopProject: (id: string) => invoke<void>('stop_project', { id }),
  restartProject: (id: string) => invoke<void>('restart_project', { id }),

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

export interface TaskbarMonitor {
  id: string;
  name: string;
  number: number;
  primary: boolean;
  hasTaskbar: boolean;
  width: number;
  height: number;
}

export interface TaskbarMonitorCatalogue {
  schemaVersion: 1;
  monitors: TaskbarMonitor[];
}

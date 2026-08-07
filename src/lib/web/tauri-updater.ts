export class Update {
  available = false;
  currentVersion = "";
  version = "";
  body?: string;
  rawJson: Record<string, unknown> = {};

  async download(): Promise<void> {}
  async install(): Promise<void> {}
  async downloadAndInstall(): Promise<void> {}
  async close(): Promise<void> {}
}

export async function check(): Promise<Update | null> {
  return null;
}

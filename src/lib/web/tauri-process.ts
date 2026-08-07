const DESKTOP =
  "Restarting the app needs the Mándalo desktop build — a browser tab has nothing to relaunch.";

export async function exit(): Promise<void> {
  throw new Error(DESKTOP);
}

export async function relaunch(): Promise<void> {
  throw new Error(DESKTOP);
}

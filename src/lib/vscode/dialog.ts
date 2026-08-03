const NO_PICKER =
  "File pickers belong to the desktop app. In VS Code, use the Explorer or the Mándalo sidebar to reach a file.";

export function open(): Promise<string> {
  return Promise.reject(new Error(NO_PICKER));
}

export function save(): Promise<string | null> {
  return Promise.reject(new Error(NO_PICKER));
}

export interface CollectionManifest {
  schemaVersion: number;
  id: string;
  name: string;
}

export interface WorkspaceManifest {
  schemaVersion: number;
  id: string;
  name: string;
}

export interface EnvironmentModel {
  name: string;
  vars: Record<string, string>;
}

export interface RequestNode {
  id: string;
  name: string;
  kind: string;
  method: string;
  /** `auth/login.http#0` — the file inside the collection plus the block index. */
  relPath: string;
  fsPath: string;
  index: number;
  line: number;
}

export interface FolderNode {
  name: string;
  relPath: string;
  folders: FolderNode[];
  requests: RequestNode[];
}

export interface CollectionNode {
  id: string;
  slug: string;
  name: string;
  dirPath: string;
  manifestPath: string;
  folders: FolderNode[];
  requests: RequestNode[];
}

export interface WorkspaceNode {
  id: string;
  name: string;
  rootPath: string;
  manifestPath: string;
  collections: CollectionNode[];
  environments: EnvironmentModel[];
  skipped: string[];
}

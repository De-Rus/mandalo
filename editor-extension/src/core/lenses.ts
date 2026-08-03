import { requestPathAt, scanTextRequests, type TextFileKind } from "./textFormat";

export interface RequestLens {
  line: number;
  title: string;
  command: string;
  relPath: string;
}

/** Two actions per `###` block, anchored on the block's own separator line. */
export function requestLenses(
  source: string,
  fileKind: TextFileKind,
  relPath: string,
): RequestLens[] {
  return scanTextRequests(source, fileKind).flatMap((block) => {
    const target = requestPathAt(relPath, block.index);
    return [
      { line: block.lineNumber, title: "▶ Send", command: "mandalo.sendRequest", relPath: target },
      {
        line: block.lineNumber,
        title: "▶ Send with env…",
        command: "mandalo.sendRequestWithEnv",
        relPath: target,
      },
    ];
  });
}

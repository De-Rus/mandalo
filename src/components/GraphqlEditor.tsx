import { BodyEditor } from "./BodyEditor";

interface GraphqlEditorProps {
  tab: "Query" | "Variables";
  query: string;
  variables: string;
  onQueryChange: (query: string) => void;
  onVariablesChange: (variables: string) => void;
}

export function GraphqlEditor({
  tab,
  query,
  variables,
  onQueryChange,
  onVariablesChange,
}: GraphqlEditorProps) {
  if (tab === "Query") {
    return (
      <textarea
        className="textarea mono fill"
        value={query}
        placeholder={"query {\n  viewer {\n    name\n  }\n}"}
        spellCheck={false}
        onChange={(e) => onQueryChange(e.target.value)}
      />
    );
  }
  return (
    <BodyEditor
      value={variables}
      onChange={onVariablesChange}
      placeholder={'{\n  "id": 1\n}'}
    />
  );
}

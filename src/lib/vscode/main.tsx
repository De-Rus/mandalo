import React from "react";
import ReactDOM from "react-dom/client";
import "../../styles/index.css";
import "./theme.css";
import { RequestEditor } from "./RequestEditor";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <RequestEditor />
  </React.StrictMode>,
);

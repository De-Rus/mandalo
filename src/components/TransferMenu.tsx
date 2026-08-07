import { useState } from "react";
import { useTransfer } from "../store/transfer";
import { Dropdown, MenuItem } from "./Dropdown";
import { ExportDialog } from "./ExportDialog";
import { Dots, Export, Import } from "./Icons";

export function TransferMenu() {
  const openImport = useTransfer((s) => s.openImport);
  const [exporting, setExporting] = useState(false);

  return (
    <>
      <Dropdown
        align="right"
        trigger={({ open: isOpen, toggle }) => (
          <button
            className={`btn-ghost btn-icon ${isOpen ? "menu-item-active" : ""}`}
            title="Import / Export"
            aria-label="Import / Export"
            onClick={toggle}
          >
            <Dots size={14} />
          </button>
        )}
      >
        {(close) => (
          <>
            <MenuItem
              icon={<Import size={13} />}
              onClick={() => {
                close();
                openImport();
              }}
            >
              Import…
            </MenuItem>
            <MenuItem
              icon={<Export size={13} />}
              onClick={() => {
                close();
                setExporting(true);
              }}
            >
              Export…
            </MenuItem>
          </>
        )}
      </Dropdown>
      {exporting && <ExportDialog onClose={() => setExporting(false)} />}
    </>
  );
}

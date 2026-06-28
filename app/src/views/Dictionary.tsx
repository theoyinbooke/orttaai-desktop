import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Badge,
  Button,
  Card,
  ConfirmDialog,
  EmptyState,
  Icon,
  PageHeader,
  Select,
  useToast,
} from "../ui";
import type { MemoryEntry } from "../types";

export default function Dictionary() {
  const [entries, setEntries] = useState<MemoryEntry[]>([]);
  const [kind, setKind] = useState("dictionary");
  const [trigger, setTrigger] = useState("");
  const [replacement, setReplacement] = useState("");
  const [pendingDelete, setPendingDelete] = useState<MemoryEntry | null>(null);
  const toast = useToast();

  const refresh = () =>
    invoke<MemoryEntry[]>("list_memory").then(setEntries).catch((e) => toast(String(e), "error"));
  useEffect(() => {
    refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function add() {
    if (!trigger.trim() || !replacement.trim()) return;
    try {
      await invoke("add_memory", { kind, trigger, replacement });
      setTrigger("");
      setReplacement("");
      refresh();
      toast("Added to your dictionary", "success");
    } catch (e) {
      toast(String(e), "error");
    }
  }

  async function confirmDelete() {
    const e = pendingDelete;
    setPendingDelete(null);
    if (!e || e.id == null) return;
    try {
      await invoke("delete_memory", { id: e.id });
      refresh();
    } catch (err) {
      toast(String(err), "error");
    }
  }

  const isSnippet = kind === "snippet";

  return (
    <div className="view stagger">
      <PageHeader
        title="Dictionary"
        desc="Fix how words are spelled, or expand short triggers into longer snippets. Applied to every transcript."
      />

      <Card className="add-form">
        <Select
          className="add-kind"
          value={kind}
          onChange={setKind}
          ariaLabel="Entry type"
          options={[
            { value: "dictionary", label: "Spelling fix" },
            { value: "snippet", label: "Snippet" },
          ]}
        />
        <input
          className="text-input"
          placeholder={isSnippet ? "trigger (e.g. addr)" : "heard as…"}
          value={trigger}
          onChange={(e) => setTrigger(e.currentTarget.value)}
          onKeyDown={(e) => e.key === "Enter" && add()}
        />
        <Icon name="arrowRight" size={16} className="add-arrow" />
        <input
          className="text-input"
          placeholder={isSnippet ? "expands to…" : "spell as…"}
          value={replacement}
          onChange={(e) => setReplacement(e.currentTarget.value)}
          onKeyDown={(e) => e.key === "Enter" && add()}
        />
        <Button variant="primary" icon="plus" onClick={add} disabled={!trigger.trim() || !replacement.trim()}>
          Add
        </Button>
      </Card>

      {entries.length === 0 ? (
        <EmptyState
          icon="dictionary"
          title="Your dictionary is empty"
          desc="Add a spelling fix (e.g. “npm” → “NPM”) or a snippet (e.g. “addr” → your address)."
        />
      ) : (
        <ul className="mem-list">
          {entries.map((e) => (
            <li key={e.id ?? e.trigger} className="mem-item">
              <Badge tone={e.kind === "snippet" ? "accent" : "neutral"}>
                {e.kind === "snippet" ? "Snippet" : "Spelling"}
              </Badge>
              <span className="mem-trigger">{e.trigger}</span>
              <Icon name="arrowRight" size={14} className="mem-arrow" />
              <span className="mem-rep">{e.replacement}</span>
              <button className="icon-btn mem-del" aria-label="Delete entry" onClick={() => setPendingDelete(e)}>
                <Icon name="trash" size={16} />
              </button>
            </li>
          ))}
        </ul>
      )}

      <ConfirmDialog
        open={pendingDelete !== null}
        title="Delete this entry?"
        body={
          pendingDelete && (
            <>
              “<strong>{pendingDelete.trigger}</strong> → {pendingDelete.replacement}” will no longer
              be applied to your transcripts. This can't be undone.
            </>
          )
        }
        confirmLabel="Delete"
        onConfirm={confirmDelete}
        onCancel={() => setPendingDelete(null)}
      />
    </div>
  );
}

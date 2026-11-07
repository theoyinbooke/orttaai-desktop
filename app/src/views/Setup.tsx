// First-run setup wizard (full-window). Owned by the engine area.
export default function Setup({ onDone }: { onDone: () => void }) {
  return (
    <div className="view">
      <button onClick={onDone}>Skip</button>
    </div>
  );
}

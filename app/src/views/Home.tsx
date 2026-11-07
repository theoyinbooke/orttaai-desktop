// Home. Owned by the analytics/home area.
import { PageHeader } from "../ui";
import type { Tab } from "../types";

export default function Home(_props: { onNavigate: (t: Tab) => void }) {
  return (
    <div className="view">
      <PageHeader title="Home" desc="Your dictation overview." />
    </div>
  );
}

// Mac HomeNavigationState.openAISettings: "AI settings" opens the Model page on
// its AI Features section. The section is handed over here because the page
// may not be mounted yet when the request arrives.
let pendingAISection = false;

export const AI_SETTINGS_ROUTE = "ai-settings";

export function requestAISection() {
  pendingAISection = true;
}

/** True once after `requestAISection`, for the Model page's first render. */
export function takeAISection(): boolean {
  const pending = pendingAISection;
  pendingAISection = false;
  return pending;
}

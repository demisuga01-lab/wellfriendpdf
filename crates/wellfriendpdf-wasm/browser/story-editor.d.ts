import type { StoryRequest, StoryWorkerClient } from "./story-client.js";
export class WellfriendStoryEditor extends HTMLElement {
  client: StoryWorkerClient;
  readonly request: StoryRequest | undefined;
  setRequest(request: StoryRequest): void;
}
declare global { interface HTMLElementTagNameMap { "wellfriend-story-editor": WellfriendStoryEditor } }

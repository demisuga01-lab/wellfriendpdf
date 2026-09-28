import { StoryWorkerClient } from "./story-client.js";
export class WellfriendScopedTextEditor extends HTMLElement {
  client: StoryWorkerClient | undefined;
}

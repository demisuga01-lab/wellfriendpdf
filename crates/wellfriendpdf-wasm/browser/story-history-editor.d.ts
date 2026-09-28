import type { StoryWorkerClient, StoryRequest } from "./story-client.js";
export class WellfriendStoryHistoryEditor extends HTMLElement {
  client: StoryWorkerClient | undefined;
  begin(base: StoryRequest): Promise<void>;
  resume(storyId:string,expectedDraft?:StoryRequest):Promise<void>;
}

export type Provider = 'codex' | 'claude';
export interface Organization { alias: string; tags: string[]; favorite: boolean; archived: boolean; removed: boolean }
export interface SessionSummary { key:string; id:string; provider:Provider; title:string; cwd:string; updatedAt:string; messageCount:number; path:string; fingerprint:string; size:number; sourceArchived:boolean; organization:Organization }
export interface Block { kind:string; text:string; id:string; name:string; input:unknown; isError:boolean }
export interface Message { role:string; timestamp:string; id:string; blocks:Block[] }
export interface Session { summary:SessionSummary; messages:Message[]; warnings:string[]; attachments:{source:string;archivePath:string;available:boolean}[];unknownRecords:number;encryptedReasoning:number;compacted:boolean }
export interface Settings { codexHome:string;claudeHome:string;codexBin:string;claudeBin:string;exportDir:string;language:string;theme:string }
export interface Capability {provider:Provider;executable:string;version:string|null;nativeVerified:boolean}
export interface MigrationPlan {sourceFingerprint:string;preserved:string[];degraded:string[];unavailable:string[];handoff:string;nativeAvailable:boolean}
export interface MigrationRequest {key:string;target:Provider;mode:'native'|'handoff';cwd:string;duplicate:boolean;handoffText?:string}
export interface MigrationResult {operationId:string;targetId:string;targetPath:string;resumeCommand:string;cwd:string;reused:boolean;warnings:string[];mode:string;provider:Provider}
export interface JobItem {request:MigrationRequest;fingerprint:string}
export interface JobEvent {jobId:string;index:number;total:number;status:string;error?:string;result?:MigrationResult;path?:string}
export interface Operation {id:string;status:string;result:string|null;createdAt:string}

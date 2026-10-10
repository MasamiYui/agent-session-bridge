import {useLayoutEffect,useRef,type RefObject} from 'react';
import {Search,Folder,ChevronRight,Command,Loader2,ArrowRightLeft} from 'lucide-react';
import type {SessionSummary,Provider} from './types';
import type {TranslationKey} from './i18n';

export function projectName(path:string){return path.split(/[\\/]/).filter(Boolean).at(-1)||path;}
export function groupProjects(sessions:SessionSummary[]){
 const groups=new Map<string,{path:string;name:string;count:number;updatedAt:string;providers:Set<Provider>}>();
 for(const s of sessions){
  if(!s.cwd||s.organization.removed||s.organization.archived)continue;
  const item=groups.get(s.cwd)||{path:s.cwd,name:projectName(s.cwd),count:0,updatedAt:s.updatedAt,providers:new Set<Provider>()};
  item.count++;item.providers.add(s.provider);
  if(s.updatedAt>item.updatedAt)item.updatedAt=s.updatedAt;
  groups.set(s.cwd,item);
 }
 return [...groups.values()].sort((a,b)=>b.updatedAt.localeCompare(a.updatedAt)||a.path.localeCompare(b.path));
}
export type Project=ReturnType<typeof groupProjects>[number];
export default function ProjectDirectory({projects,query,onQuery,onOpen,onSync,syncing,scroll,searchRef,loading,t,date}:{projects:Project[];query:string;onQuery:(value:string)=>void;onOpen:(path:string)=>void;onSync:(path:string)=>void;syncing:boolean;scroll:RefObject<number>;searchRef:RefObject<HTMLInputElement|null>;loading:boolean;t:(key:TranslationKey)=>string;date:(value:string)=>string}){
 const listRef=useRef<HTMLDivElement>(null);
 useLayoutEffect(()=>{if(listRef.current)listRef.current.scrollTop=scroll.current},[scroll]);
 const matching=projects.filter(p=>p.path.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase()));
 return <>
  <div className="panel-heading directory-heading"><div><h1>{t('projects')}</h1><span>{projects.length} {t('projectCount')}</span></div><Folder size={20}/></div>
  <div className="search-box"><Search size={16}/><input ref={searchRef} value={query} onChange={e=>{scroll.current=0;if(listRef.current)listRef.current.scrollTop=0;onQuery(e.target.value)}} placeholder={t('searchProjects')} aria-label={t('searchProjects')}/><kbd>{t('keyboard')}</kbd></div>
  <div className="directory-caption">{t('recentActivity')}{query&&<span>{matching.length} / {projects.length}</span>}</div>
  <div className="project-list" ref={listRef} onScroll={e=>{scroll.current=e.currentTarget.scrollTop}}>
   {loading?<div className="list-empty"><Loader2 size={22} className="spin"/>{t('loading')}</div>:matching.length===0?<div className="list-empty"><Folder size={24}/><strong>{t(projects.length?'noProjectsFound':'noProjects')}</strong><p>{t(projects.length?'noProjectsFoundBody':'noProjectsBody')}</p></div>:matching.map(p=><div className="project-row" key={p.path}><button className="project-open" title={p.path} onClick={()=>onOpen(p.path)}><div className="project-row-heading"><Folder size={17}/><strong className="truncate">{p.name}</strong><ChevronRight size={15}/></div><p className="project-path truncate">{p.path}</p><div className="project-row-meta"><span>{p.count} {t('sessions')}</span><span className="project-providers">{p.providers.has('codex')&&<span title="Codex" aria-label="Codex"><Command size={12}/></span>}{p.providers.has('claude')&&<span className="claude-source" title="Claude Code" aria-label="Claude Code">✳</span>}{p.providers.has('antigravity')&&<span className="antigravity-source" title="Antigravity" aria-label="Antigravity">A</span>}</span><time>{date(p.updatedAt)}</time></div></button><button className="project-sync-button" title={t('syncProject')} aria-label={`${t('syncProject')} ${p.name}`} disabled={syncing} onClick={()=>onSync(p.path)}><ArrowRightLeft size={13}/>{t('sync')}</button></div>)}
  </div>
 </>;
}

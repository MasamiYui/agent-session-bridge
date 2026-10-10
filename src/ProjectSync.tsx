import {ArrowRightLeft,Check,AlertCircle,Loader2} from 'lucide-react';
import type {TranslationKey} from './i18n';
import type {ProjectSyncPlan} from './types';

export default function ProjectSync({plan,loading,t}:{plan:ProjectSyncPlan|null;loading:boolean;t:(key:TranslationKey)=>string}){
 if(loading)return <div className="plan-loading"><Loader2 size={18} className="spin"/>{t('syncChecking')}</div>;
 if(!plan)return null;
 return <>
  <p className="sync-path" title={plan.cwd}>{plan.cwd}</p>
  <p className="subtle-note">{t('syncHelp')}</p>
  <p className="subtle-note">{t('syncVersionHelp')}</p>
  <div className="sync-directions">{(['claude','codex'] as const).map(source=>{
   const items=plan.items.filter(item=>item.source===source);
   const pending=items.filter(item=>item.action==='create');
   return <div key={source}><strong>{source==='claude'?'Claude Code':'Codex'} <ArrowRightLeft size={13}/> {source==='claude'?'Codex':'Claude Code'}</strong><p>{pending.length} {t('syncPending')} / {items.length} {t('sessions')}</p></div>;
  })}</div>
  {plan.items.some(item=>item.mode==='handoff'&&item.action==='create')&&<p className="sync-caveat"><AlertCircle size={15}/>{t('syncHandoffHelp')}</p>}
  {plan.items.some(item=>item.mode==='native'&&item.action==='create')&&<p className="subtle-note">{t('syncNativeHelp')}</p>}
  <div className="sync-items">{plan.items.map(item=><div className="sync-item" key={item.key}>
   {item.action==='create'?<ArrowRightLeft size={14}/>:<Check size={14}/>}
   <div><strong>{item.title}</strong><span>{item.source==='claude'?'Claude Code → Codex':'Codex → Claude Code'} · {t(item.action==='skip'?'syncSkipped':item.mode==='native'?'native':'handoff')}{item.reason&&` · ${t(item.reason==='generated'?'syncGenerated':item.reason==='synced'?'syncSynced':'syncRecovery')}`}</span></div>
  </div>)}</div>
  {plan.items.every(item=>item.action==='skip')&&<p className="subtle-note">{t('syncUpToDate')}</p>}
 </>;
}

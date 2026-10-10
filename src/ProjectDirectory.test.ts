import {describe,it,expect} from 'vitest';
import {groupProjects,projectName} from './ProjectDirectory';
import type {SessionSummary} from './types';

const summary=(cwd:string,updatedAt:string,patch:Partial<SessionSummary>={}):SessionSummary=>({key:cwd,id:cwd,provider:'codex',title:'Session',cwd,updatedAt,messageCount:1,path:'session.jsonl',fingerprint:'test',size:100,sourceArchived:false,organization:{alias:'',tags:[],favorite:false,archived:false,removed:false},...patch});
describe('project directory',()=>{
 it('groups both sources by full path and orders projects by their latest session',()=>{
  const projects=groupProjects([
   summary('/work/bridge','2026-10-01T12:00:00Z'),
   summary('/work/bridge','2026-10-03T12:00:00Z',{provider:'claude'}),
   summary('/other/bridge','2026-10-02T12:00:00Z'),
  ]);
  expect(projects.map(p=>p.path)).toEqual(['/work/bridge','/other/bridge']);
  expect(projects[0].count).toBe(2);
  expect([...projects[0].providers]).toEqual(['codex','claude']);
  expect(groupProjects([summary('/work/bridge','2026-10-04T12:00:00Z',{provider:'antigravity'})])[0].providers.has('antigravity')).toBe(true);
  expect(projects[0].updatedAt).toBe('2026-10-03T12:00:00Z');
 });
 it('only counts sessions that can be reached in the project view',()=>{
  const base=summary('/work/bridge','2026-10-01T12:00:00Z');
  const projects=groupProjects([base,{...base,organization:{...base.organization,archived:true}}, {...base,cwd:'/removed',organization:{...base.organization,removed:true}}, {...base,cwd:''}]);
  expect(projects).toHaveLength(1);
  expect(projects[0].count).toBe(1);
 });
 it('extracts directory names on Windows, Unix and root paths',()=>{
  expect(projectName('C:\\work\\bridge\\')).toBe('bridge');
  expect(projectName('/work/bridge/')).toBe('bridge');
  expect(projectName('/')).toBe('/');
 });
});

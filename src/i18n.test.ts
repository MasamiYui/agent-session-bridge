import {describe,it,expect,vi} from 'vitest';
import {translator,displayTitle} from './i18n';
describe('language and titles',()=>{
 it('uses the system language and respects explicit overrides',()=>{vi.stubGlobal('navigator',{language:'zh-CN'});expect(translator('system')('import')).toBe('导入');expect(translator('en')('import')).toBe('Import');vi.unstubAllGlobals()});
 it('keeps source title visible unless a Bridge alias is set',()=>{expect(displayTitle({title:'Original',organization:{alias:''}})).toBe('Original');expect(displayTitle({title:'Original',organization:{alias:'My name'}})).toBe('My name')});
});

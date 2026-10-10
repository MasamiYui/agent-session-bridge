import {test,expect} from '@playwright/test';
test.beforeEach(async({page})=>{await page.goto('/');await expect(page.getByText('Trace the cache invalidation race',{exact:true})).toBeVisible();});
test('search, preview and expand tool results',async({page})=>{
 await page.getByRole('textbox',{name:'Search conversations…'}).fill('cache');
 await expect(page.locator('.session-row')).toHaveCount(1);
 await page.locator('.session-select').click();
 await expect(page.getByRole('heading',{name:'Trace the cache invalidation race',level:2})).toBeVisible();
 await page.locator('.tool-block.result summary').click();
 await expect(page.locator('.tool-block.result pre')).toContainText('18 checks passed');
});
test('organize and restore without removing source history',async({page})=>{
 await page.locator('.session-select').first().click();
 await page.getByTitle('Organize').click();
 await page.getByLabel('Display name',{exact:true}).fill('My investigation');
 await page.getByRole('button',{name:'Save',exact:true}).click();
 await expect(page.getByRole('heading',{name:'My investigation',level:2})).toBeVisible();
 await page.getByTitle('Archive',{exact:true}).click();
 await page.getByRole('button',{name:'Archive',exact:true}).click();
 await expect(page.getByText('My investigation',{exact:true}).first()).toBeVisible();
 await page.getByTitle('Restore',{exact:true}).click();
 await page.getByRole('button',{name:/All sessions/}).click();
 await expect(page.getByText('My investigation',{exact:true}).first()).toBeVisible();
});
test('switch language and theme',async({page})=>{
 await page.getByRole('button',{name:'Settings',exact:true}).click();
 await page.getByLabel('Language',{exact:true}).selectOption('zh');
 await page.getByLabel('Theme',{exact:true}).selectOption('dark');
 await page.getByRole('button',{name:'Save',exact:true}).click();
 await expect(page.getByRole('button',{name:'设置',exact:true})).toBeVisible();
 await expect(page.locator('html')).toHaveAttribute('data-theme','dark');
});
test('batch handoff reports completion and blocks unverified native conversion',async({page})=>{
 await page.getByRole('button',{name:'Select visible',exact:true}).click();
 await page.getByTitle('Bridge selected').click();
 await expect(page.getByRole('button',{name:/Native session/})).toBeDisabled();
 await expect(page.getByRole('textbox',{name:'Handoff document',exact:true})).toBeVisible();
 await page.getByRole('button',{name:'Create and continue',exact:true}).click();
 await expect(page.locator('.job-row')).toHaveCount(5);
 await expect(page.locator('.job-row').last()).toContainText('Completed');
});

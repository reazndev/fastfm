import { TestBed } from '@angular/core/testing';

import { ConfirmButton } from './confirm-button';

describe('ConfirmButton', () => {
  it('emits only after the second click', async () => {
    const fixture = TestBed.createComponent(ConfirmButton);
    fixture.componentRef.setInput('label', 'Revoke');
    fixture.componentRef.setInput('question', 'Sure?');
    fixture.componentRef.setInput('confirmLabel', 'Revoke key');
    let confirmed = 0;
    fixture.componentInstance.confirmed.subscribe(() => confirmed++);
    await fixture.whenStable();
    const element = fixture.nativeElement as HTMLElement;
    element.querySelector('button')?.click();
    await fixture.whenStable();
    expect(confirmed).toBe(0);
    expect(element.textContent).toContain('Sure?');
    element.querySelector<HTMLButtonElement>('button.danger')?.click();
    await fixture.whenStable();
    expect(confirmed).toBe(1);
    expect(element.textContent).not.toContain('Sure?');
  });
});

import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { Button } from './Button';
import { Callout } from './Callout';
import { Pill } from './Panel';
import { TextField } from './TextField';

describe('Button', () => {
  it('invokes its handler when activated', async () => {
    const onClick = vi.fn();
    render(<Button onClick={onClick}>Publish</Button>);

    await userEvent.click(screen.getByRole('button', { name: 'Publish' }));
    expect(onClick).toHaveBeenCalledOnce();
  });

  it('defaults to type="button" so it cannot accidentally submit a form', () => {
    render(<Button>Cancel</Button>);
    expect(screen.getByRole('button')).toHaveAttribute('type', 'button');
  });

  it('is disabled and marked busy while loading', async () => {
    const onClick = vi.fn();
    render(
      <Button isLoading loadingLabel="Signing in" onClick={onClick}>
        Sign in
      </Button>,
    );

    const button = screen.getByRole('button');
    expect(button).toBeDisabled();
    expect(button).toHaveAttribute('aria-busy', 'true');
    expect(button).toHaveAccessibleName('Signing in');

    // A double-submit must not reach the handler.
    await userEvent.click(button);
    expect(onClick).not.toHaveBeenCalled();
  });

  it('keeps its label when no loading label is supplied', () => {
    render(<Button isLoading>Save</Button>);
    expect(screen.getByRole('button')).toHaveAccessibleName('Save');
  });
});

describe('TextField', () => {
  it('associates its visible label with the input', () => {
    render(<TextField label="Username" required value="" onChange={() => undefined} />);
    // Found by label, which only works if `for`/`id` are wired up.
    expect(screen.getByLabelText('Username')).toBeInTheDocument();
  });

  it('marks optional fields in words rather than only with an attribute', () => {
    render(<TextField label="Note" value="" onChange={() => undefined} />);
    expect(screen.getByLabelText(/Note.*optional/s)).toBeInTheDocument();
  });

  it('exposes the hint as the input description', () => {
    render(
      <TextField
        label="Username"
        hint="Letters, digits, dots, underscores, and hyphens."
        required
        value=""
        onChange={() => undefined}
      />,
    );
    expect(screen.getByLabelText('Username')).toHaveAccessibleDescription(/Letters, digits/);
  });

  it('marks itself invalid and announces the error when one is present', () => {
    render(
      <TextField
        label="Password"
        error="Password must be at least 12 characters"
        required
        value=""
        onChange={() => undefined}
      />,
    );

    const input = screen.getByLabelText('Password');
    expect(input).toHaveAttribute('aria-invalid', 'true');
    expect(input).toHaveAccessibleDescription(/at least 12 characters/);
    // role=alert is what gets the message read out even after focus has moved.
    expect(screen.getByRole('alert')).toHaveTextContent('at least 12 characters');
  });

  it('is not marked invalid when there is no error', () => {
    render(<TextField label="Username" required value="" onChange={() => undefined} />);
    expect(screen.getByLabelText('Username')).not.toHaveAttribute('aria-invalid');
  });

  it('renders the live region up front so a later error is announced', () => {
    // Screen readers do not reliably announce content inserted at the same moment
    // the live region itself appears, so the region must already exist.
    const { container } = render(
      <TextField label="Username" required value="" onChange={() => undefined} />,
    );
    expect(container.querySelector('[role="alert"]')).not.toBeNull();
  });

  it('reports every keystroke to its handler', async () => {
    const onChange = vi.fn();
    render(<TextField label="Username" required value="" onChange={onChange} />);

    await userEvent.type(screen.getByLabelText('Username'), 'abc');
    expect(onChange).toHaveBeenCalledTimes(3);
  });
});

describe('Callout', () => {
  it('interrupts for a failure but waits its turn otherwise', () => {
    const { unmount } = render(
      <Callout tone="danger" title="Could not sign in">
        Invalid credentials
      </Callout>,
    );
    expect(screen.getByRole('alert')).toHaveTextContent('Invalid credentials');
    unmount();

    render(<Callout tone="info">Nothing has happened yet</Callout>);
    expect(screen.getByRole('status')).toHaveTextContent('Nothing has happened yet');
  });

  it('names its tone in text, so colour is never the only signal', () => {
    render(
      <Callout tone="caution" title="Too many attempts">
        Try again in 300 seconds.
      </Callout>,
    );
    expect(screen.getByText('Too many attempts')).toBeInTheDocument();
  });
});

describe('Pill', () => {
  it('always renders a word rather than a bare colour', () => {
    render(<Pill tone="success">Active</Pill>);
    expect(screen.getByText('Active')).toBeInTheDocument();
  });
});

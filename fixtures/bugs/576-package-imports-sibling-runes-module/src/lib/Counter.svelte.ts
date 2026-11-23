export class CounterState {
    count = $state(0);
}

export function formatCount(n: number): string {
    return String(n);
}

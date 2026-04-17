export function useThing(kind: string): () => void {
  return () => {
    void kind
  }
}

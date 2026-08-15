export function match(param: string): param is 'grid' | 'list' {
    return param === 'grid' || param === 'list';
}

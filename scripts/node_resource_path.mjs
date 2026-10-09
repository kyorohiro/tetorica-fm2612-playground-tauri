// Mirror the native launcher: Node 22 cannot resolve a main module with a
// verbatim drive prefix, even when the entry point is relative to a verbatim cwd.
export function nodeResourcePath(path){
  if(path.startsWith('\\\\?\\UNC\\'))return '\\\\'+path.slice(8);
  if(/^\\\\\?\\[A-Za-z]:\\/.test(path))return path.slice(4);
  return path;
}

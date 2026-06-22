/* M35.2 - FILENAME global statement plus INCLUDE non-quoted forms.
   The real fileref-to-file success path is covered by unit tests (the snapshot
   harness cannot materialise an included file). This fixture locks the new
   observable boundary behaviour: a FILENAME device keyword is noted and
   ignored; a stdin star include is deferred; an unknown bare token is reported
   as not readable. */
filename scratch temp;
%put step one;
%include nosuchref;
%put step two;

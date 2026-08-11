/* M38.5 - FILENAME device filerefs (PIPE/URL) are recognised and deferred with
   a clean NOTE at the statement, and INCLUDE of such a fileref (or of star =
   keyboard input) is consumed with a clean deferral NOTE instead of failing
   silently. The session continues and exits 0. The plain disk FILENAME form is
   unchanged (covered by m35 and unit tests). The DATA _NULL_ step provides the
   segment boundary that makes the fileref visible to the later INCLUDE. */
filename mypipe pipe 'ls -l';
filename web url 'https://example.com/prog.sas';
data _null_;
run;
%put step one;
%include *;
%include mypipe;
%put step two;

/* M41.1/M41.2: %QUOTE/%NRQUOTE (new) alongside the already-implemented
   %BQUOTE/%NRBQUOTE/%SUPERQ family, exercised together since M41 audited
   the whole quoting set for SAS 9.4 fidelity.
   NOTE: comments are scanned by the macro processor too (documented
   simplification, see M12) - avoid writing anything resembling a real
   percent-triggered construct inside a comment in this file. */

%let x = Z;
%let w = %nrstr(&x);
%let v = %quote(a;b);

data q;
  length q1 q2 q3 q4 q5 q6 $12;
  /* quote resolves x first then masks the semicolon so it stays literal */
  q1 = "%quote(&x;y)";
  /* an unpaired open paren, escaped so the call still bounds correctly */
  q2 = "%quote(a%(b)";
  /* nrquote also masks the still-undefined z reference in the result */
  q3 = "%nrquote(a&z b)";
  /* superq never resolves - w holds the literal two characters x-ref */
  q4 = "%superq(w)";
  /* bquote behaves like quote here, without requiring percent escapes */
  q5 = "%bquote(&x;y)";
  /* nrbquote also masks the undefined z reference, like nrquote */
  q6 = "%nrbquote(a&z b)";
run;

data qv;
  length v $6;
  /* v was assigned above from a quote call whose masked semicolon did not
     end the surrounding let statement */
  v = "&v";
run;

title 'M41 quoting: %QUOTE/%NRQUOTE + %BQUOTE/%NRBQUOTE/%SUPERQ';
proc print data=q noobs;
  var q1 q2 q3 q4 q5 q6;
run;

proc print data=qv noobs;
  var v;
run;

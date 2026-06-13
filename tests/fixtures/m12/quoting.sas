/* M12: macro string and quoting functions in assignments. */
%let raw = abcdef;

data q;
  up      = "%qupcase(&raw)";
  part    = "%substr(&raw, 2, 3)";
  scanned = "%scan(a.b.c, 2, .)";
  len     = %length(&raw);
run;

title 'Macro string and quoting functions';
proc print data=q;
  var up part scanned len;
run;

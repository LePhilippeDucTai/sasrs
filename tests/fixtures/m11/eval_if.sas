/* M11: integer eval and a conditional inside a macro body; q is 7/2=3, pick(7,2) gives big=7. */
%macro pick(a, b);
  %if &a > &b %then %do; big = &a; %end;
  %else %do; big = &b; %end;
%mend;

data calc;
  q = %eval(7 / 2);
  %pick(7, 2)
run;

title 'Macro %eval and %if inside a DATA step';
proc print data=calc;
  var q big;
run;

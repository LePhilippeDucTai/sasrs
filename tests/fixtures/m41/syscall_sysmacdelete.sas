/* M41.3: %SYSCALL SORTN/SORTC (in-place sort of macro variables) and
   %SYSMACDELETE (removes a compiled macro definition). */

%let a = 30;
%let b = 10;
%let c = 20;
%syscall sortn(a, b, c);

%let x = banana;
%let y = apple;
%let z = cherry;
%syscall sortc(x, y, z);

data nums;
  length n1 n2 n3 $4 c1 c2 c3 $8;
  n1 = "&a"; n2 = "&b"; n3 = "&c";
  c1 = "&x"; c2 = "&y"; c3 = "&z";
run;

title '%SYSCALL SORTN/SORTC: values sorted ascending in place';
proc print data=nums noobs;
run;

%macro greet;
hello
%mend;

%put Before delete: [%greet];
%sysmacdelete greet;
%put After delete: [%greet];

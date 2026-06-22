/* M35.1 : %SYSFUNC delegates to the full DATA-step function library (no more
   whitelist) and supports the optional trailing format.
   Oracles:
     reverse(abcde)            -> edcba
     sqrt(144)                 -> 12
     propcase(hello world)     -> Hello World
     mdy(7,4,1776) , date9.    -> 04JUL1776
     sum(1000,234.5), dollar10.2 -> $1,234.50
     unknown function          -> clean ERROR */
%let r = %sysfunc(reverse(abcde));
%let s = %sysfunc(sqrt(144));
%let m = %sysfunc(propcase(hello world));
%let d = %sysfunc(mdy(7,4,1776), date9.);
%let cash = %sysfunc(sum(1000, 234.5), dollar10.2);
%put r=&r;
%put s=&s;
%put m=&m;
%put d=&d;
%put cash=&cash;
%put bad=%sysfunc(no_such_fn(1));

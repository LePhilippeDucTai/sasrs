/* M6 : PROC SQL — SELECT avec WHERE, GROUP BY, agregats, ORDER BY. */
libname d 'data';

title 'Average height/weight by sex (age 13+)';
proc sql;
  select sex,
         count(*) as n,
         avg(height) as avg_h,
         avg(weight) as avg_w
  from d.class
  where age >= 13
  group by sex
  order by sex;
quit;

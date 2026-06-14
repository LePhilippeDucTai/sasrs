/* M15.3 : Date/time functions — DATEPART, TIMEPART, DATETIME, HMS,  */
/* DHMS, YRDIF, DATDIF, JULDATE, DATEJUL, HOUR, MINUTE, SECOND, NLDATE.*/

data work.test;
  /* DATEPART/TIMEPART — decompose datetime */
  dt = dhms(21710, 14, 30, 45);  /* 2020-01-01 14:30:45 */
  dp = datepart(dt);             /* Should be 21710 (SAS date) */
  tp = timepart(dt);             /* Should be ~52245 (14:30:45 in seconds) */

  /* DATETIME/HMS/DHMS — construct datetime */
  time_hms = hms(14, 30, 45);    /* Should be ~52245 seconds */
  dt_rebuild = datetime(21710, time_hms);  /* Rebuild datetime */

  /* HMS edge cases */
  time_midnight = hms(0, 0, 0);  /* Should be 0 */
  time_end = hms(23, 59, 59);    /* Should be 86399 */

  /* JULDATE — day of year */
  sas_date_1 = 21710;            /* 2020-01-01 */
  jul_1 = juldate(sas_date_1);   /* Should be 1 */
  sas_date_2 = 21710 + 364;      /* 2020-12-30 (leap year) */
  jul_2 = juldate(sas_date_2);   /* Should be 365 */

  /* DATEJUL — inverse */
  dj_20001 = datejul(20001);     /* year 2000, day 1 = 2000-01-01 */
  dj_07365 = datejul(07365);     /* year 2007, day 365 */

  /* YRDIF/DATDIF — differences */
  date1 = 21710;                 /* 2020-01-01 */
  date2 = 21710 + 365;           /* 2021-01-01 (1 year later) */
  yrdif_actual = yrdif(date1, date2, 'ACTUAL');  /* Should be ~1.0 */
  datdif_actual = datdif(date1, date2, 'ACTUAL');  /* Should be 365 */

  /* HOUR/MINUTE/SECOND — extract time components */
  dt_test = dhms(21710, 14, 30, 45);  /* 2020-01-01 14:30:45 */
  hr = hour(dt_test);            /* Should be 14 */
  mn = minute(dt_test);          /* Should be 30 */
  sc = second(dt_test);          /* Should be 45 */

  /* NLDATE — format as text */
  nl_en = nldate(21710, 'EN');   /* Should be "01JAN2020" */
  nl_default = nldate(21710);    /* Default EN: "01JAN2020" */

  output;
run;

title 'M15.3 Date/Time Functions';
proc print data=work.test;
run;

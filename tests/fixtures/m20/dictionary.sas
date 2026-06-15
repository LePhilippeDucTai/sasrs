/* M20.3 — dictionary tables pour PROC SQL */

/* Test DICTIONARY.TABLES et sashelp.vtable */
proc sql;
    title "DICTIONARY.TABLES — lister tous les datasets";
    select libname, memname, nobs, nvar
    from dictionary.tables
    order by memname;

    title "sashelp.vtable (alias) — même requête";
    select libname, memname, nobs, nvar
    from sashelp.vtable
    order by memname;
quit;

/* Test DICTIONARY.COLUMNS et sashelp.vcolumn */
proc sql;
    title "DICTIONARY.COLUMNS — variables du dataset CLASS";
    select name, type, length, varnum
    from dictionary.columns
    where memname = 'CLASS'
    order by varnum;

    title "sashelp.vcolumn (alias) — même requête";
    select name, type, length, varnum
    from sashelp.vcolumn
    where memname = 'CLASS'
    order by varnum;
quit;

/* Test DICTIONARY.MACROS et sashelp.vmacro */
%let MYVAR = hello;
proc sql;
    title "DICTIONARY.MACROS — variables macro globales";
    select scope, name, value
    from dictionary.macros
    where name like 'MY%'
    order by name;

    title "sashelp.vmacro (alias) — même requête";
    select scope, name, value
    from sashelp.vmacro
    where name like 'MY%'
    order by name;
quit;

/* Test WHERE clause sur dictionary tables */
proc sql;
    title "DICTIONARY.COLUMNS avec WHERE";
    select name, type, length
    from dictionary.columns
    where memname = 'CLASS' and type = 'num'
    order by name;
quit;

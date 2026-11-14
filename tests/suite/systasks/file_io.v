// File I/O (IEEE 1800 21.3): $fopen modes, $fdisplay/$fwrite, $fgets, $fscanf, $feof,
// $fclose, $sscanf, $sformat/$sformatf, multichannel descriptors
module t;
  integer errors = 0;
  integer fd, n, a, b, c;
  reg [8*20:1] line;
  reg [8*10:1] word;
  string s;
  task check(input integer line_no, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d want %0d", line_no, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    fd = $fopen("out.txt", "w");
    $fdisplay(fd, "12 34 ab");
    $fwrite(fd, "second %0d\n", 7);
    $fclose(fd);
    fd = $fopen("out.txt", "r");
    n = $fgets(line, fd);
    check(`__LINE__, n, 9);
    if (line[8*9:1] != "12 34 ab\n") begin $display("FAILED line %0d", `__LINE__); errors++; end
    n = $fscanf(fd, "%s %d", word, a);
    check(`__LINE__, n, 2);
    check(`__LINE__, a, 7);
    if (word[8*6:1] != "second") begin $display("FAILED line %0d", `__LINE__); errors++; end
    n = $fgetc(fd);                  // the newline
    n = $fgetc(fd);
    check(`__LINE__, n, -1);         // EOF
    check(`__LINE__, $feof(fd) != 0, 1);
    $fclose(fd);
    fd = $fopen("out.txt", "a");
    $fdisplay(fd, "third");
    $fclose(fd);
    fd = $fopen("out.txt", "r");
    n = 0;
    while ($fgets(line, fd)) n++;
    check(`__LINE__, n, 3);
    $fclose(fd);
    fd = $fopen("missing/none.txt", "r");
    check(`__LINE__, fd, 0);
    n = $sscanf("10 1f 077", "%d %h %o", a, b, c);
    check(`__LINE__, n, 3);
    check(`__LINE__, a, 10);
    check(`__LINE__, b, 31);
    check(`__LINE__, c, 63);
    n = $sscanf("1x", "%h", a);      // x is a hex digit
    check(`__LINE__, a[7:0], 8'h1x);
    n = $sscanf("ff 17", "%h %o", a, b);
    check(`__LINE__, a, 255);
    check(`__LINE__, b, 15);
    $sformat(line, "%0d-%h", 5, 8'hab);
    if (line[8*4:1] != "5-ab") begin $display("FAILED line %0d", `__LINE__); errors++; end
    s = $sformatf("%03d|%-4s|", 7, "x");
    if (s != "007|x   |") begin $display("FAILED line %0d: %s", `__LINE__, s); errors++; end
    fd = $fopen("mc.txt");           // multichannel: bit 0 is stdout
    check(`__LINE__, fd[0], 0);
    check(`__LINE__, fd > 1, 1);
    $fclose(fd);
    if (errors == 0) $display("PASSED");
  end
endmodule

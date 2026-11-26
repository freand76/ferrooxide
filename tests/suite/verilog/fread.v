// $fread (IEEE 1800-2017 21.3.4.4) reads bytes, the first one the most significant, into
// a variable (ceil(w/8) bytes, truncated to w bits) or into array elements from a start
// address, at most count of them; bits no byte reaches keep their value; the result is the
// number of bytes read (0 for a bad descriptor or start). $ferror (21.3.7) gives the
// code and text of the last file error.
module t;
  integer fd, res, err;
  reg [8:0] rg;
  reg [15:0] w16;
  reg [7:0] mem [31:0];
  reg [15:0] m16 [0:1];
  string s;
  reg [8*40:1] rs;
  reg ok = 1;
  task check(input c, input [8*16:1] what);
    if (!c) begin
      $display("FAILED: %0s", what);
      ok = 0;
    end
  endtask
  initial begin
    fd = $fopen("fread_tmp.txt", "w");
    err = $ferror(fd, s);
    check(err == 0 && s == "", "ferror ok");
    $fwrite(fd, "ab01zy0123k");
    $fclose(fd);
    fd = $fopen("no_such_file_here.txt", "r");
    err = $ferror(fd, s);
    check(err != 0 && s != "", "ferror fail");
    err = $ferror(fd, rs);
    check(err != 0 && rs != 0, "ferror reg");
    rg = 9'bx;
    res = $fread(rg, fd);
    check(res == 0 && rg === 9'bx, "bad fd");
    fd = $fopen("fread_tmp.txt", "r");
    res = $fread(mem, fd, -1);
    check(res == 0 && mem[0] === 8'bx, "bad start");
    res = $fread(rg, fd);
    check(res == 2 && rg === ("ab" & 9'h1ff), "reg");
    res = $fread(mem, fd, 0, 2);
    check(res == 2 && mem[0] == "0" && mem[1] == "1" && mem[2] === 8'bx, "mem count");
    res = $fread(mem, fd, 31);
    check(res == 1 && mem[31] == "z", "mem start");
    res = $fread(mem, fd, 31, 2);
    check(res == 1 && mem[31] == "y", "mem past end");
    res = $fread(m16, fd);
    check(res == 4 && m16[0] == "01" && m16[1] == "23", "wide elements");
    w16 = 16'bx;
    res = $fread(w16, fd);
    check(res == 1 && w16 === {"k", 8'bx}, "partial");
    res = $fread(w16, fd);
    check(res == 0 && $feof(fd), "eof");
    $fclose(fd);
    if (ok) $display("PASSED");
  end
endmodule

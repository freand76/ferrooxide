// expected
// $ftell, $fseek and $rewind on a file opened for writing; a write after a seek
// overwrites from the new position.
module test;
  integer fd, r, c;
  initial begin
    fd = $fopen("fseek_write.txt", "w");
    $display("tell %0d", $ftell(fd));
    $fwrite(fd, "abcdefghij");
    $display("tell %0d", $ftell(fd));
    r = $fseek(fd, 3, 0);
    $display("seek %0d tell %0d", r, $ftell(fd));
    $fwrite(fd, "XY");
    $display("tell %0d", $ftell(fd));
    r = $fseek(fd, -1, 2);
    $display("seek %0d tell %0d", r, $ftell(fd));
    r = $fseek(fd, -2, 1);
    $display("seek %0d tell %0d", r, $ftell(fd));
    r = $rewind(fd);
    $display("tell %0d", $ftell(fd));
    $fclose(fd);
    fd = $fopen("fseek_write.txt", "r");
    c = $fgetc(fd);
    while (c != -1) begin
      $write("%c", c);
      c = $fgetc(fd);
    end
    $display("");
    $fclose(fd);
  end
endmodule

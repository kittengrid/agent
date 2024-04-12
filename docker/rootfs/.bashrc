# ~/.bashrc: executed by bash(1) for non-login shells.
# see /usr/share/doc/bash/examples/startup-files (in the package bash-doc)
# for examples

# FIX USER NEEDED SO WE CAN SHARE UID BETWEEN HOST AND DEV ENV
function delete_entry() {
    local type=$1
    local name=$2
    local unless=$3

    if ent=$(getent "${type}" "${name}") && ! echo $ent | grep $unless > /dev/null; then
        cat "/etc/${type}" | grep -v "${ent}" > "/tmp/new_${type}" && cp "/tmp/new_${type}" "/etc/${type}" && rm "/tmp/new_${type}"
    fi
}

# Ensures the user passed has uid/gid of the current process
function fix_user() {
    local user=$1
    GID=$(id -g)

    # If user is ok, we return
    [ "$(id -u ${user})" = $UID ] && [ "$(id -g ${user})" = $GID ] && return 1

    delete_entry passwd $UID $user
    delete_entry group $GID $user
    usermod -o -u $(id -u) $user
    groupmod -o -g $(id -g) $user

    sudo chmod ugo-s /usr/sbin/usermod /usr/sbin/groupmod /usr/bin/cp # We fix the setuid in those commands as we no have sudo

    return 0
}

fix_user kittengrid

# don't put duplicate lines or lines starting with space in the history.
# See bash(1) for more options
HISTCONTROL=ignoreboth

# append to the history file, don't overwrite it
shopt -s histappend

# for setting history length see HISTSIZE and HISTFILESIZE in bash(1)
HISTSIZE=1000
HISTFILESIZE=2000

# check the window size after each command and, if necessary,
# update the values of LINES and COLUMNS.
shopt -s checkwinsize

PS1='\[\e]0;dev@kittengrid-contaner\h: \w\a\]${debian_chroot:+($debian_chroot)}\[\033[01;32m\]dev\[\033[00m\]@\[\033[01;32m\]kittengrid-container\[\033[00m\]:\[\033[01;34m\]\w\[\033[00m\]\[\033[01;31m\]$(git branch &>/dev/null; if [ $? -eq 0 ]; then echo " ($(git branch | grep ^* |sed s/\*\ //))"; fi)\[\033[00m\]\$ '

# Alias definitions.
# You may want to put all your additions into a separate file like
# ~/.bash_aliases, instead of adding them here directly.
# See /usr/share/doc/bash-doc/examples in the bash-doc package.

if [ -f ~/.bash_aliases ]; then
  . ~/.bash_aliases
fi

# enable programmable completion features (you don't need to enable
# this, if it's already enabled in /etc/bash.bashrc and /etc/profile
# sources /etc/bash.bashrc).
if ! shopt -oq posix; then
  if [ -f /usr/share/bash-completion/bash_completion ]; then
    . /usr/share/bash-completion/bash_completion
  elif [ -f /etc/bash_completion ]; then
    . /etc/bash_completion
  fi
fi

export HOME=/home/kittengrid
export PATH=/usr/local/cargo/bin:$PATH
